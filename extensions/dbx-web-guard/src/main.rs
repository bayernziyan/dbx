mod config;
mod credentials;
mod dpapi;
mod policy;
mod session;
mod web;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use config::GuardConfig;
use credentials::{read_confirmed_password, CredentialStore, Role};

#[derive(Debug, Parser)]
#[command(name = "dbx-web-guard", version, about = "External dual-role access guard for DBX Web")]
struct Cli {
    #[arg(long, env = "DBX_WEB_GUARD_CONFIG")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve,
    Password {
        #[command(subcommand)]
        command: PasswordCommand,
    },
    Credential {
        #[command(subcommand)]
        command: CredentialCommand,
    },
    Check,
}

#[derive(Debug, Subcommand)]
enum PasswordCommand {
    Set {
        #[arg(long)]
        role: String,
    },
}

#[derive(Debug, Subcommand)]
enum CredentialCommand {
    SetUpstream,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "dbx_web_guard=info".into()),
        )
        .with_target(false)
        .compact()
        .init();
    let cli = Cli::parse();
    let config = GuardConfig::load(&cli.config)?;
    let credentials = CredentialStore::open(config.storage.credentials_db.clone())?;
    match cli.command {
        Command::Serve => {
            let password = load_upstream_password(&config)?;
            let state = web::AppState::build(config, credentials, password).await?;
            web::serve(state).await
        }
        Command::Password { command: PasswordCommand::Set { role } } => {
            let role = Role::parse(&role)?;
            let password = read_confirmed_password(&format!("New {} password: ", role.as_str()))?;
            credentials.set_password(role, &password)?;
            println!("{} password hash updated", role.as_str());
            Ok(())
        }
        Command::Credential { command: CredentialCommand::SetUpstream } => {
            let password = read_confirmed_password("Upstream DBX password: ")?;
            verify_upstream_password(&config, &password).await?;
            dpapi::protect_to_file(&config.upstream.credential_file, password.as_bytes())?;
            println!("upstream credential encrypted and stored");
            Ok(())
        }
        Command::Check => {
            if !credentials.configured()? {
                bail!("admin and viewer passwords are not both configured");
            }
            let password = load_upstream_password(&config)?;
            verify_upstream_password(&config, &password).await?;
            if !config.static_files.directory.is_dir() {
                bail!("static directory does not exist");
            }
            println!("guard configuration is ready");
            Ok(())
        }
    }
}

fn load_upstream_password(config: &GuardConfig) -> Result<String> {
    let bytes = dpapi::unprotect_from_file(&config.upstream.credential_file)?;
    String::from_utf8(bytes).context("upstream credential is not UTF-8")
}

async fn verify_upstream_password(config: &GuardConfig, password: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(config.upstream.connect_timeout_seconds))
        .timeout(std::time::Duration::from_secs(config.upstream.request_timeout_seconds))
        .build()
        .context("build upstream verification client")?;
    let url = config.upstream_url(&config.public_path("/api/auth/login"));
    let response = client
        .post(url)
        .json(&serde_json::json!({"password":password}))
        .send()
        .await
        .context("connect to upstream DBX login")?;
    if !response.status().is_success() {
        bail!("UPSTREAM_AUTH_FAILED: status {}", response.status());
    }
    let has_session = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.split(';').next().is_some_and(|pair| pair.trim().starts_with("dbx_session=")));
    if !has_session {
        bail!("UPSTREAM_AUTH_FAILED: DBX did not issue a session");
    }
    Ok(())
}
