use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::credentials::Role;

#[derive(Debug, Clone)]
pub struct GuardSession {
    pub role: Role,
    pub upstream_cookie: String,
    pub created_at: Instant,
    pub last_seen: Instant,
}

#[derive(Debug)]
pub struct SessionStore {
    sessions: RwLock<HashMap<String, GuardSession>>,
    idle_timeout: Duration,
    absolute_timeout: Duration,
}

impl SessionStore {
    pub fn new(idle_timeout: Duration, absolute_timeout: Duration) -> Self {
        Self { sessions: RwLock::new(HashMap::new()), idle_timeout, absolute_timeout }
    }

    pub async fn create(&self, role: Role, upstream_cookie: String) -> String {
        let token = Uuid::new_v4().to_string();
        let now = Instant::now();
        self.sessions
            .write()
            .await
            .insert(token.clone(), GuardSession { role, upstream_cookie, created_at: now, last_seen: now });
        token
    }

    pub async fn get(&self, token: &str) -> Option<GuardSession> {
        let now = Instant::now();
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(token)?;
        if now.duration_since(session.created_at) >= self.absolute_timeout
            || now.duration_since(session.last_seen) >= self.idle_timeout
        {
            sessions.remove(token);
            return None;
        }
        session.last_seen = now;
        Some(session.clone())
    }

    pub async fn update_upstream_cookie(&self, token: &str, upstream_cookie: String) {
        if let Some(session) = self.sessions.write().await.get_mut(token) {
            session.upstream_cookie = upstream_cookie;
            session.last_seen = Instant::now();
        }
    }

    pub async fn remove(&self, token: &str) -> Option<GuardSession> {
        self.sessions.write().await.remove(token)
    }

    pub async fn invalidate_role_except(&self, role: Role, keep: Option<&str>) {
        self.sessions.write().await.retain(|token, session| session.role != role || keep == Some(token.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::SessionStore;
    use crate::credentials::Role;

    #[tokio::test]
    async fn sessions_are_independent_and_expire() {
        let store = SessionStore::new(Duration::from_millis(20), Duration::from_secs(1));
        let admin = store.create(Role::Admin, "dbx_session=admin".to_string()).await;
        let viewer = store.create(Role::Viewer, "dbx_session=viewer".to_string()).await;
        assert_ne!(admin, viewer);
        assert_eq!(store.get(&admin).await.unwrap().upstream_cookie, "dbx_session=admin");
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(store.get(&viewer).await.is_none());
    }
}
