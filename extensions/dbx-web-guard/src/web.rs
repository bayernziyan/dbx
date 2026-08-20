use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, ORIGIN, SET_COOKIE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use axum::Router;
use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use reqwest::redirect::Policy;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{info, warn};
use zeroize::Zeroizing;

use crate::config::GuardConfig;
use crate::credentials::{CredentialStore, Role};
use crate::policy::ViewerPolicy;
use crate::session::{GuardSession, SessionStore};

const BOOTSTRAP_JS: &str = include_str!("../assets/bootstrap.js");
const VIEWER_CSS: &str = include_str!("../assets/viewer.css");

#[derive(Debug)]
pub struct AppState {
    pub config: GuardConfig,
    pub credentials: CredentialStore,
    pub upstream_password: Arc<Zeroizing<String>>,
    pub sessions: SessionStore,
    pub policy: ViewerPolicy,
    pub client: reqwest::Client,
    login_throttle: Mutex<HashMap<String, LoginThrottle>>,
}

#[derive(Debug, Clone)]
struct LoginThrottle {
    failures: u32,
    locked_until: Option<Instant>,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    password: String,
}

#[derive(Debug, Deserialize)]
struct ChangePasswordRequest {
    old_password: String,
    new_password: String,
}

impl AppState {
    pub async fn build(
        config: GuardConfig,
        credentials: CredentialStore,
        upstream_password: String,
    ) -> Result<Arc<Self>> {
        if !credentials.configured()? {
            bail!("both admin and viewer passwords must be configured before serve");
        }
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(config.upstream.connect_timeout_seconds))
            .timeout(Duration::from_secs(config.upstream.request_timeout_seconds))
            .build()
            .context("build upstream HTTP client")?;
        let state = Arc::new(Self {
            sessions: SessionStore::new(
                Duration::from_secs(config.session.idle_timeout_minutes.saturating_mul(60)),
                Duration::from_secs(config.session.absolute_timeout_minutes.saturating_mul(60)),
            ),
            policy: ViewerPolicy::from_config(&config.policy),
            config,
            credentials,
            upstream_password: Arc::new(Zeroizing::new(upstream_password)),
            client,
            login_throttle: Mutex::new(HashMap::new()),
        });
        upstream_login(&state).await.context("verify upstream DBX credential")?;
        Ok(state)
    }
}

pub async fn serve(state: Arc<AppState>) -> Result<()> {
    let address = state.config.server.listen;
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(address).await.with_context(|| format!("bind Guard on {address}"))?;
    info!(%address, "dbx-web-guard ready");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve Guard")
}

fn router(state: Arc<AppState>) -> Router {
    Router::new().fallback(any(dispatch)).with_state(state)
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        warn!(%error, "failed to listen for shutdown signal");
    }
}

async fn dispatch(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request<Body>,
) -> Response<Body> {
    match dispatch_inner(state, peer, request).await {
        Ok(response) => response,
        Err(error) => {
            warn!(error = %format!("{error:#}"), "request failed");
            json_error(StatusCode::BAD_GATEWAY, "WEB_GUARD_UPSTREAM_ERROR", "The DBX service is unavailable.")
        }
    }
}

async fn dispatch_inner(state: Arc<AppState>, peer: SocketAddr, request: Request<Body>) -> Result<Response<Body>> {
    let path = request.uri().path().to_string();
    let base = &state.config.server.public_base_path;
    if path == "/__guard/health/live" {
        return Ok(json(StatusCode::OK, serde_json::json!({"status":"live"})));
    }
    if path == "/__guard/health/ready" {
        return readiness(state).await;
    }
    if base != "/" && path == *base {
        let location = format!("{base}/");
        return Ok(Response::builder()
            .status(StatusCode::PERMANENT_REDIRECT)
            .header("location", location)
            .body(Body::empty())?);
    }
    let relative = if base == "/" {
        path.strip_prefix('/')
    } else {
        path.strip_prefix(base).and_then(|value| value.strip_prefix('/'))
    };
    let Some(relative) = relative else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    if relative == "__guard/bootstrap.js" {
        return Ok(static_text("application/javascript; charset=utf-8", BOOTSTRAP_JS));
    }
    if relative == "__guard/viewer.css" {
        return Ok(static_text("text/css; charset=utf-8", VIEWER_CSS));
    }
    if relative.starts_with("api/auth/") {
        return handle_auth(state, peer, request, relative).await;
    }
    if relative.starts_with("api/") {
        return handle_api(state, request).await;
    }
    serve_static(&state.config, relative).await
}

async fn readiness(state: Arc<AppState>) -> Result<Response<Body>> {
    let credential_ok = state.credentials.configured().unwrap_or(false);
    let static_ok = state.config.static_files.directory.is_dir();
    let upstream_ok = upstream_login(&state).await.is_ok();
    let ready = credential_ok && static_ok && upstream_ok;
    Ok(json(
        if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        serde_json::json!({"status": if ready {"ready"} else {"not_ready"}}),
    ))
}

async fn handle_auth(
    state: Arc<AppState>,
    peer: SocketAddr,
    request: Request<Body>,
    relative: &str,
) -> Result<Response<Body>> {
    match (request.method().clone(), relative) {
        (Method::POST, "api/auth/login") => login(state, peer, request).await,
        (Method::GET, "api/auth/check") => auth_check(state, request.headers()).await,
        (Method::POST, "api/auth/logout") => logout(state, request.headers()).await,
        (Method::POST, "api/auth/change-password") => change_password(state, request).await,
        (Method::POST, "api/auth/setup") => Ok(forbidden()),
        _ => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

async fn login(state: Arc<AppState>, peer: SocketAddr, request: Request<Body>) -> Result<Response<Body>> {
    if !origin_allowed(&state.config, request.headers(), request.method()) {
        return Ok(json_error(StatusCode::FORBIDDEN, "WEB_GUARD_ORIGIN_FORBIDDEN", "Origin is not allowed."));
    }
    let key = client_key(request.headers(), peer);
    if login_locked(&state, &key).await {
        return Ok(json_error(StatusCode::TOO_MANY_REQUESTS, "WEB_GUARD_LOGIN_LOCKED", "Please try again later."));
    }
    let bytes = to_bytes(request.into_body(), state.config.max_body_bytes()).await.context("read login request")?;
    let body: LoginRequest = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(_) => return Ok(json_error(StatusCode::BAD_REQUEST, "WEB_GUARD_LOGIN_INVALID", "Invalid login request.")),
    };
    let role = state.credentials.identify(&body.password)?;
    let Some(role) = role else {
        record_login_failure(&state, &key).await;
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    clear_login_failure(&state, &key).await;
    let upstream_cookie = upstream_login(&state).await?;
    let token = state.sessions.create(role, upstream_cookie).await;
    info!(role = role.as_str(), "Guard login succeeded");
    let mut response = json(StatusCode::OK, serde_json::json!({"ok":true}));
    response.headers_mut().append(SET_COOKIE, HeaderValue::from_str(&session_cookie(&state.config, &token))?);
    response.headers_mut().append(SET_COOKIE, HeaderValue::from_str(&ui_cookie(&state.config, role))?);
    Ok(response)
}

async fn auth_check(state: Arc<AppState>, headers: &HeaderMap) -> Result<Response<Body>> {
    let authenticated = match cookie_value(headers, &state.config.session.cookie_name) {
        Some(token) => state.sessions.get(&token).await.is_some(),
        None => false,
    };
    Ok(json(StatusCode::OK, serde_json::json!({"authenticated":authenticated,"required":true,"setup_required":false})))
}

async fn logout(state: Arc<AppState>, headers: &HeaderMap) -> Result<Response<Body>> {
    if let Some(token) = cookie_value(headers, &state.config.session.cookie_name) {
        if let Some(session) = state.sessions.remove(&token).await {
            let _ = upstream_logout(&state, &session.upstream_cookie).await;
        }
    }
    let mut response = json(StatusCode::OK, serde_json::json!({"ok":true}));
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_cookie(&state.config, &state.config.session.cookie_name, true))?,
    );
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_cookie(&state.config, &state.config.session.ui_cookie_name, false))?,
    );
    Ok(response)
}

async fn change_password(state: Arc<AppState>, request: Request<Body>) -> Result<Response<Body>> {
    if !origin_allowed(&state.config, request.headers(), request.method()) {
        return Ok(forbidden());
    }
    let Some((token, session)) = authenticated_session(&state, request.headers()).await else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if session.role != Role::Admin {
        return Ok(forbidden());
    }
    let bytes = to_bytes(request.into_body(), state.config.max_body_bytes()).await.context("read password change")?;
    let body: ChangePasswordRequest = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(_) => {
            return Ok(json_error(
                StatusCode::BAD_REQUEST,
                "WEB_GUARD_PASSWORD_INVALID",
                "Invalid password change request.",
            ));
        }
    };
    if !state.credentials.verify(Role::Admin, &body.old_password)? {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }
    if state.credentials.set_password(Role::Admin, &body.new_password).is_err() {
        return Ok(json_error(
            StatusCode::BAD_REQUEST,
            "WEB_GUARD_PASSWORD_REJECTED",
            "The new password does not satisfy Guard policy.",
        ));
    }
    state.sessions.invalidate_role_except(Role::Admin, Some(&token)).await;
    Ok(json(StatusCode::OK, serde_json::json!({"ok":true})))
}

async fn handle_api(state: Arc<AppState>, request: Request<Body>) -> Result<Response<Body>> {
    let websocket = is_websocket_upgrade(request.headers());
    if !origin_allowed(&state.config, request.headers(), request.method())
        || (websocket && !websocket_origin_allowed(&state.config, request.headers()))
    {
        return Ok(json_error(StatusCode::FORBIDDEN, "WEB_GUARD_ORIGIN_FORBIDDEN", "Origin is not allowed."));
    }
    let Some((token, session)) = authenticated_session(&state, request.headers()).await else {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    };
    if session.role == Role::Viewer && !state.policy.allows(request.method(), request.uri().path()) {
        warn!(method = %request.method(), path = request.uri().path(), "viewer request denied");
        return Ok(forbidden());
    }
    if websocket {
        return proxy_websocket(state, session, request).await;
    }
    proxy_request(state, token, session, request).await
}

async fn proxy_websocket(
    state: Arc<AppState>,
    session: GuardSession,
    mut request: Request<Body>,
) -> Result<Response<Body>> {
    let downstream_upgrade = hyper::upgrade::on(&mut request);
    let upstream_url = url::Url::parse(&state.config.upstream.base_url).context("parse upstream URL")?;
    let host = upstream_url.host_str().context("upstream host is missing")?;
    let port = upstream_url.port_or_known_default().context("upstream port is missing")?;
    let stream = tokio::net::TcpStream::connect((host, port)).await.context("connect upstream WebSocket")?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .context("start upstream WebSocket handshake")?;
    tokio::spawn(async move {
        if let Err(error) = connection.with_upgrades().await {
            warn!(%error, "upstream WebSocket connection failed");
        }
    });

    let mut builder = hyper::Request::builder()
        .method(request.method())
        .uri(request.uri().path_and_query().map(|value| value.as_str()).unwrap_or(request.uri().path()))
        .version(axum::http::Version::HTTP_11);
    for (name, value) in request.headers() {
        if name != COOKIE && !name.as_str().eq_ignore_ascii_case("x-dbx-guard-role") {
            builder = builder.header(name, value);
        }
    }
    let upstream_request =
        builder.header("host", format!("{host}:{port}")).header(COOKIE, &session.upstream_cookie).body(Body::empty())?;
    let mut upstream_response = sender.send_request(upstream_request).await.context("upstream WebSocket handshake")?;
    if upstream_response.status() != StatusCode::SWITCHING_PROTOCOLS {
        let status = upstream_response.status();
        let headers = upstream_response.headers().clone();
        let body = Body::from_stream(upstream_response.into_body().into_data_stream());
        let mut response = Response::builder().status(status);
        for (name, value) in &headers {
            if forward_response_header(name) {
                response = response.header(name, value);
            }
        }
        return Ok(response.body(body)?);
    }
    let upstream_upgrade = hyper::upgrade::on(&mut upstream_response);
    let mut response = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);
    for (name, value) in upstream_response.headers() {
        if forward_websocket_response_header(name) {
            response = response.header(name, value);
        }
    }
    tokio::spawn(async move {
        let result = async {
            let downstream = downstream_upgrade.await.context("upgrade browser WebSocket")?;
            let upstream = upstream_upgrade.await.context("upgrade upstream WebSocket")?;
            let mut downstream = TokioIo::new(downstream);
            let mut upstream = TokioIo::new(upstream);
            tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await.context("relay WebSocket")?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            warn!(%error, "WebSocket relay ended with error");
        }
    });
    Ok(response.body(Body::empty())?)
}

async fn authenticated_session(state: &AppState, headers: &HeaderMap) -> Option<(String, GuardSession)> {
    let token = cookie_value(headers, &state.config.session.cookie_name)?;
    let session = state.sessions.get(&token).await?;
    Some((token, session))
}

async fn proxy_request(
    state: Arc<AppState>,
    token: String,
    mut session: GuardSession,
    request: Request<Body>,
) -> Result<Response<Body>> {
    let (parts, body) = request.into_parts();
    let mut body = to_bytes(body, state.config.max_body_bytes()).await.context("read proxied request body")?;
    let viewer = session.role == Role::Viewer;
    if viewer && parts.method == Method::POST && parts.uri.path() == state.config.public_path("/api/ai/agent-stream") {
        let template = match load_first_prompt_template(&state, &session.upstream_cookie).await {
            Ok(template) => template,
            Err(_) => {
                return Ok(json_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "WEB_GUARD_TEMPLATE_UNAVAILABLE",
                    "The required prompt template is unavailable.",
                ));
            }
        };
        body = match force_agent_mode(&body, template.as_ref()) {
            Ok(body) => body,
            Err(_) => {
                return Ok(json_error(
                    StatusCode::BAD_REQUEST,
                    "WEB_GUARD_AI_REQUEST_INVALID",
                    "Invalid AI Agent request.",
                ));
            }
        };
    }
    let mut upstream =
        send_upstream(&state, &parts.method, &parts.uri, &parts.headers, &session.upstream_cookie, body.clone())
            .await?;
    if upstream.status() == reqwest::StatusCode::UNAUTHORIZED {
        let new_cookie = upstream_login(&state).await?;
        state.sessions.update_upstream_cookie(&token, new_cookie.clone()).await;
        session.upstream_cookie = new_cookie;
        upstream =
            send_upstream(&state, &parts.method, &parts.uri, &parts.headers, &session.upstream_cookie, body).await?;
    }
    if viewer && parts.method == Method::GET && parts.uri.path() == state.config.public_path("/api/prompt-templates") {
        return first_prompt_template_response(upstream).await;
    }
    upstream_response(upstream)
}

fn force_agent_mode(body: &[u8], template: Option<&serde_json::Value>) -> Result<Bytes> {
    let mut value: serde_json::Value = serde_json::from_slice(body).context("parse AI Agent request")?;
    let object = value.as_object_mut().context("AI Agent request must be a JSON object")?;
    object.insert("mode".to_string(), serde_json::Value::String("agent".to_string()));
    if let Some(request) = object.get_mut("request").and_then(serde_json::Value::as_object_mut) {
        if let Some(contract) = request.get_mut("taskContract").and_then(serde_json::Value::as_object_mut) {
            contract.insert("mode".to_string(), serde_json::Value::String("agent".to_string()));
        }
        if let Some(template) = template {
            let name = template.get("name").and_then(serde_json::Value::as_str).unwrap_or("Default");
            let content = template.get("content").and_then(serde_json::Value::as_str).unwrap_or("").trim();
            if !content.is_empty() {
                let block = format!("### {name}\n{content}");
                let prompt = request.entry("systemPrompt").or_insert_with(|| serde_json::Value::String(String::new()));
                let prompt = prompt.as_str().context("request.systemPrompt must be a string")?.to_string();
                if !prompt.contains(&block) {
                    request.insert(
                        "systemPrompt".to_string(),
                        serde_json::Value::String(format!("{prompt}\n\n## Guard-enforced viewer template\n{block}")),
                    );
                }
            }
        }
    }
    Ok(Bytes::from(serde_json::to_vec(&value).context("serialize AI Agent request")?))
}

async fn load_first_prompt_template(state: &AppState, upstream_cookie: &str) -> Result<Option<serde_json::Value>> {
    let url = state.config.upstream_url(&state.config.public_path("/api/prompt-templates"));
    let response =
        state.client.get(url).header(COOKIE, upstream_cookie).send().await.context("load prompt templates")?;
    if !response.status().is_success() {
        bail!("prompt template request failed with status {}", response.status());
    }
    let templates: Vec<serde_json::Value> = response.json().await.context("parse prompt templates")?;
    Ok(templates.into_iter().next())
}

async fn first_prompt_template_response(upstream: reqwest::Response) -> Result<Response<Body>> {
    if !upstream.status().is_success() {
        return upstream_response(upstream);
    }
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let bytes = upstream.bytes().await.context("read prompt templates response")?;
    let body = first_prompt_template_body(&bytes)?;
    let mut builder = Response::builder().status(status);
    for (name, value) in &headers {
        if forward_transformed_response_header(name) {
            builder = builder.header(name, value);
        }
    }
    Ok(builder.header(CONTENT_TYPE, "application/json").body(Body::from(body))?)
}

fn first_prompt_template_body(body: &[u8]) -> Result<Vec<u8>> {
    let mut templates: Vec<serde_json::Value> =
        serde_json::from_slice(body).context("parse prompt templates response")?;
    templates.truncate(1);
    serde_json::to_vec(&templates).context("serialize filtered prompt templates")
}

async fn send_upstream(
    state: &AppState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    upstream_cookie: &str,
    body: Bytes,
) -> Result<reqwest::Response> {
    let path_and_query = uri.path_and_query().map(|value| value.as_str()).unwrap_or(uri.path());
    let mut builder = state.client.request(method.clone(), state.config.upstream_url(path_and_query));
    for (name, value) in headers {
        if forward_request_header(name) {
            builder = builder.header(name, value);
        }
    }
    builder = builder.header(COOKIE, upstream_cookie).body(body);
    builder.send().await.context("proxy request to DBX Web")
}

fn upstream_response(upstream: reqwest::Response) -> Result<Response<Body>> {
    let status = upstream.status();
    let mut builder = Response::builder().status(status);
    for (name, value) in upstream.headers() {
        if forward_response_header(name) {
            builder = builder.header(name, value);
        }
    }
    Ok(builder.body(Body::from_stream(upstream.bytes_stream()))?)
}

async fn upstream_login(state: &AppState) -> Result<String> {
    let url = state.config.upstream_url(&state.config.public_path("/api/auth/login"));
    let response = state
        .client
        .post(url)
        .json(&serde_json::json!({"password": state.upstream_password.as_str()}))
        .send()
        .await
        .context("connect to upstream login")?;
    if !response.status().is_success() {
        bail!("UPSTREAM_AUTH_FAILED: status {}", response.status());
    }
    extract_upstream_cookie(response.headers()).context("upstream login did not return dbx_session")
}

async fn upstream_logout(state: &AppState, cookie: &str) -> Result<()> {
    let url = state.config.upstream_url(&state.config.public_path("/api/auth/logout"));
    state.client.post(url).header(COOKIE, cookie).send().await.context("upstream logout")?;
    Ok(())
}

fn extract_upstream_cookie(headers: &HeaderMap) -> Option<String> {
    headers.get_all(SET_COOKIE).iter().filter_map(|value| value.to_str().ok()).find_map(|value| {
        value.split(';').next().map(str::trim).filter(|pair| pair.starts_with("dbx_session=")).map(str::to_string)
    })
}

async fn serve_static(config: &GuardConfig, relative: &str) -> Result<Response<Body>> {
    let decoded = percent_decode_str(relative).decode_utf8().context("decode static path")?;
    let mut relative_path = PathBuf::new();
    for component in Path::new(decoded.as_ref()).components() {
        match component {
            Component::Normal(value) => relative_path.push(value),
            Component::CurDir => {}
            _ => return Ok(StatusCode::NOT_FOUND.into_response()),
        }
    }
    let root = tokio::fs::canonicalize(&config.static_files.directory)
        .await
        .with_context(|| format!("resolve static root {}", config.static_files.directory.display()))?;
    let index = root.join(&config.static_files.index_file);
    let candidate = if relative_path.as_os_str().is_empty() { index.clone() } else { root.join(&relative_path) };
    let path = if candidate.is_file() {
        let resolved = tokio::fs::canonicalize(&candidate)
            .await
            .with_context(|| format!("resolve static file {}", candidate.display()))?;
        if !resolved.starts_with(&root) {
            return Ok(StatusCode::NOT_FOUND.into_response());
        }
        resolved
    } else {
        index
    };
    let bytes = tokio::fs::read(&path).await.with_context(|| format!("read static file {}", path.display()))?;
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    if mime.type_() == mime_guess::mime::TEXT && mime.subtype() == mime_guess::mime::HTML {
        let html = String::from_utf8(bytes).context("index.html must be UTF-8")?;
        return Ok(static_text("text/html; charset=utf-8", &inject_html(&html, &config.server.public_base_path)));
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, mime.as_ref())
        .header(CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(bytes))?)
}

fn inject_html(html: &str, base: &str) -> String {
    let asset_base = if base == "/" { "" } else { base };
    let injection = format!(
        "<link rel=\"stylesheet\" href=\"{asset_base}/__guard/viewer.css\"><script defer src=\"{asset_base}/__guard/bootstrap.js\"></script>"
    );
    if let Some(index) = html.rfind("</head>") {
        let mut output = String::with_capacity(html.len() + injection.len());
        output.push_str(&html[..index]);
        output.push_str(&injection);
        output.push_str(&html[index..]);
        output
    } else {
        format!("{injection}{html}")
    }
}

fn origin_allowed(config: &GuardConfig, headers: &HeaderMap, method: &Method) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    if let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok()) {
        return config.security.allowed_origins.iter().any(|allowed| allowed == origin);
    }
    headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .map(|value| matches!(value, "same-origin" | "same-site" | "none"))
        .unwrap_or(true)
}

fn websocket_origin_allowed(config: &GuardConfig, headers: &HeaderMap) -> bool {
    headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| config.security.allowed_origins.iter().any(|allowed| allowed == origin))
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get("upgrade")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    raw.split(';').map(str::trim).find_map(|pair| pair.strip_prefix(&format!("{name}=")).map(str::to_string))
}

fn cookie_suffix(config: &GuardConfig, http_only: bool) -> String {
    format!(
        "; Path={}; SameSite=Strict{}{}",
        config.server.public_base_path,
        if http_only { "; HttpOnly" } else { "" },
        if config.session.secure_cookies { "; Secure" } else { "" }
    )
}

fn session_cookie(config: &GuardConfig, token: &str) -> String {
    format!("{}={}{}", config.session.cookie_name, token, cookie_suffix(config, true))
}

fn ui_cookie(config: &GuardConfig, role: Role) -> String {
    format!("{}={}{}", config.session.ui_cookie_name, role.as_str(), cookie_suffix(config, false))
}

fn clear_cookie(config: &GuardConfig, name: &str, http_only: bool) -> String {
    format!("{name}=; Max-Age=0{}", cookie_suffix(config, http_only))
}

fn forward_request_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str().to_ascii_lowercase().as_str(),
        "host"
            | "cookie"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-dbx-guard-role"
            | "accept-encoding"
    )
}

fn forward_response_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str().to_ascii_lowercase().as_str(),
        "set-cookie" | "connection" | "keep-alive" | "transfer-encoding" | "upgrade"
    )
}

fn forward_transformed_response_header(name: &HeaderName) -> bool {
    forward_response_header(name)
        && !matches!(name.as_str().to_ascii_lowercase().as_str(), "content-length" | "content-encoding")
}

fn forward_websocket_response_header(name: &HeaderName) -> bool {
    !matches!(name.as_str().to_ascii_lowercase().as_str(), "set-cookie" | "x-dbx-guard-role")
}

fn static_text(content_type: &'static str, content: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .header(CACHE_CONTROL, "no-cache")
        .body(Body::from(content.to_string()))
        .expect("static response")
}

fn json(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .header(CACHE_CONTROL, "no-store")
        .body(Body::from(value.to_string()))
        .expect("JSON response")
}

fn json_error(status: StatusCode, code: &str, message: &str) -> Response<Body> {
    json(status, serde_json::json!({"code":code,"error":message}))
}

fn forbidden() -> Response<Body> {
    json_error(StatusCode::FORBIDDEN, "WEB_GUARD_VIEWER_FORBIDDEN", "This operation requires an administrator session.")
}

fn client_key(headers: &HeaderMap, peer: SocketAddr) -> String {
    headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.parse::<std::net::IpAddr>().is_ok())
        .map(str::to_string)
        .unwrap_or_else(|| peer.ip().to_string())
}

async fn login_locked(state: &AppState, key: &str) -> bool {
    let mut entries = state.login_throttle.lock().await;
    let Some(entry) = entries.get_mut(key) else { return false };
    if entry.locked_until.is_some_and(|until| until > Instant::now()) {
        return true;
    }
    if entry.locked_until.is_some() {
        entry.failures = 0;
        entry.locked_until = None;
    }
    false
}

async fn record_login_failure(state: &AppState, key: &str) {
    let mut entries = state.login_throttle.lock().await;
    let entry = entries.entry(key.to_string()).or_insert(LoginThrottle { failures: 0, locked_until: None });
    entry.failures = entry.failures.saturating_add(1);
    if entry.failures >= state.config.security.login_max_attempts {
        entry.failures = 0;
        entry.locked_until = Some(Instant::now() + Duration::from_secs(state.config.security.login_lockout_seconds));
    }
}

async fn clear_login_failure(state: &AppState, key: &str) {
    state.login_throttle.lock().await.remove(key);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{HeaderMap, HeaderValue, Method};
    use axum::response::Response;
    use axum::routing::{any, get, post};
    use axum::{Json, Router};

    use super::{
        cookie_value, first_prompt_template_body, force_agent_mode, inject_html, origin_allowed, router, AppState,
    };
    use crate::config::{
        GuardConfig, PolicyConfig, PolicyRuleConfig, SecurityConfig, ServerConfig, SessionConfig, StaticConfig,
        StorageConfig, UpstreamConfig,
    };
    use crate::credentials::{CredentialStore, Role};

    fn config() -> GuardConfig {
        GuardConfig {
            server: ServerConfig { listen: "127.0.0.1:0".parse().unwrap(), public_base_path: "/dbx".to_string() },
            upstream: UpstreamConfig {
                base_url: "http://127.0.0.1:4225".to_string(),
                credential_file: "credential".into(),
                connect_timeout_seconds: 5,
                request_timeout_seconds: 30,
            },
            static_files: StaticConfig { directory: ".".into(), index_file: "index.html".to_string() },
            storage: StorageConfig { credentials_db: "credentials.db".into() },
            session: SessionConfig::default(),
            security: SecurityConfig {
                allowed_origins: vec!["http://server:82".to_string()],
                ..SecurityConfig::default()
            },
            policy: PolicyConfig::default(),
        }
    }

    async fn spawn_upstream() -> String {
        let counter = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/dbx/api/auth/login",
                post(|State(counter): State<Arc<AtomicUsize>>| async move {
                    let id = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    Response::builder()
                        .status(200)
                        .header("set-cookie", format!("dbx_session=upstream-{id}; Path=/dbx; HttpOnly"))
                        .body(Body::from("{\"ok\":true}"))
                        .unwrap()
                }),
            )
            .route(
                "/dbx/api/ai/agent-stream",
                any(|headers: HeaderMap| async move {
                    Json(serde_json::json!({"cookie": headers.get("cookie").and_then(|value| value.to_str().ok())}))
                }),
            )
            .route(
                "/dbx/api/prompt-templates",
                get(|| async { Json(serde_json::json!([{"id":"first","name":"First","content":"Use Wiki first."}])) }),
            )
            .route("/dbx/api/update/check", any(|| async { "admin-upstream" }))
            .with_state(counter);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    fn guard_cookie(headers: &reqwest::header::HeaderMap) -> String {
        headers
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(|value| {
                value.split(';').next().filter(|pair| pair.starts_with("dbx_guard_session=")).map(str::to_string)
            })
            .unwrap()
    }

    #[test]
    fn injects_guard_assets_before_head_close() {
        let output = inject_html("<html><head></head><body></body></html>", "/dbx");
        assert!(output.contains("/dbx/__guard/bootstrap.js"));
        assert!(output.find("bootstrap.js").unwrap() < output.find("</head>").unwrap());
        let root = inject_html("<html><head></head></html>", "/");
        assert!(root.contains("src=\"/__guard/bootstrap.js\""));
        assert!(!root.contains("//__guard"));
    }

    #[test]
    fn extracts_only_named_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_static("other=1; dbx_guard_session=abc"));
        assert_eq!(cookie_value(&headers, "dbx_guard_session").as_deref(), Some("abc"));
    }

    #[test]
    fn enforces_origin_for_unsafe_browser_requests() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static("http://server:82"));
        assert!(origin_allowed(&config, &headers, &Method::POST));
        headers.insert("origin", HeaderValue::from_static("http://evil"));
        assert!(!origin_allowed(&config, &headers, &Method::POST));
        assert!(origin_allowed(&config, &headers, &Method::GET));
    }

    #[test]
    fn viewer_agent_request_is_forced_to_agent_mode() {
        for original in ["ask", "agent"] {
            let body = serde_json::to_vec(&serde_json::json!({
                "mode":original,
                "instruction":"query",
                "request":{"systemPrompt":"base","taskContract":{"mode":original}}
            }))
            .unwrap();
            let template = serde_json::json!({"name":"First","content":"Always use the Wiki first."});
            let forced: serde_json::Value =
                serde_json::from_slice(&force_agent_mode(&body, Some(&template)).unwrap()).unwrap();
            assert_eq!(forced["mode"], "agent");
            assert_eq!(forced["instruction"], "query");
            assert_eq!(forced["request"]["taskContract"]["mode"], "agent");
            assert!(forced["request"]["systemPrompt"].as_str().unwrap().contains("### First"));
        }
        assert!(force_agent_mode(b"[]", None).is_err());
    }

    #[test]
    fn viewer_prompt_template_response_keeps_only_first_item() {
        let input = serde_json::to_vec(&serde_json::json!([
            {"id":"first","name":"First"},
            {"id":"second","name":"Second"}
        ]))
        .unwrap();
        let output: Vec<serde_json::Value> =
            serde_json::from_slice(&first_prompt_template_body(&input).unwrap()).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["id"], "first");
    }

    #[tokio::test]
    async fn dual_roles_use_independent_upstream_sessions_and_viewer_cannot_spoof_admin() {
        let upstream = spawn_upstream().await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<html><head></head><body></body></html>").unwrap();
        let credentials = CredentialStore::open(dir.path().join("credentials.db")).unwrap();
        credentials.set_password(Role::Admin, "admin-password-1").unwrap();
        credentials.set_password(Role::Viewer, "viewer-password-1").unwrap();
        let mut config = config();
        config.upstream.base_url = upstream;
        config.static_files.directory = dir.path().to_path_buf();
        config.policy = PolicyConfig {
            viewer_default_deny: true,
            viewer_allow: vec![PolicyRuleConfig {
                path: "/dbx/api/ai/agent-stream".to_string(),
                exact: true,
                methods: vec!["POST".to_string()],
            }],
            viewer_deny: vec![PolicyRuleConfig {
                path: "/dbx/api/update/".to_string(),
                exact: false,
                methods: vec!["*".to_string()],
            }],
        };
        let state = AppState::build(config, credentials, "upstream-service-password".to_string()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router(state).into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await
                .unwrap()
        });
        let client = reqwest::Client::new();
        let login = |password: &'static str| {
            client
                .post(format!("http://{address}/dbx/api/auth/login"))
                .header("origin", "http://server:82")
                .json(&serde_json::json!({"password":password}))
        };
        let admin_login = login("admin-password-1").send().await.unwrap();
        let viewer_login = login("viewer-password-1").send().await.unwrap();
        assert!(admin_login.status().is_success());
        assert!(viewer_login.status().is_success());
        assert!(admin_login
            .headers()
            .get_all("set-cookie")
            .iter()
            .all(|value| !value.to_str().unwrap().contains("dbx_session=")));
        let admin_cookie = guard_cookie(admin_login.headers());
        let viewer_cookie = guard_cookie(viewer_login.headers());
        assert_ne!(admin_cookie, viewer_cookie);

        let viewer_ai = client
            .post(format!("http://{address}/dbx/api/ai/agent-stream"))
            .header("origin", "http://server:82")
            .header("cookie", format!("{viewer_cookie}; dbx_guard_ui=admin"))
            .json(&serde_json::json!({
                "mode":"ask",
                "request":{"systemPrompt":"base","taskContract":{"mode":"ask"}}
            }))
            .send()
            .await
            .unwrap();
        assert!(viewer_ai.status().is_success());
        let upstream_cookie =
            viewer_ai.json::<serde_json::Value>().await.unwrap()["cookie"].as_str().unwrap().to_string();
        assert!(upstream_cookie.starts_with("dbx_session=upstream-"));

        let denied = client
            .get(format!("http://{address}/dbx/api/update/check"))
            .header("cookie", format!("{viewer_cookie}; dbx_guard_ui=admin"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);

        let allowed = client
            .get(format!("http://{address}/dbx/api/update/check"))
            .header("cookie", admin_cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(allowed.text().await.unwrap(), "admin-upstream");
    }
}
