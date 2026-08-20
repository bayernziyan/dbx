use axum::http::Method;

use crate::config::{PolicyConfig, PolicyRuleConfig};

#[derive(Debug, Clone)]
pub struct ViewerPolicy {
    default_deny: bool,
    allow: Vec<Rule>,
    deny: Vec<Rule>,
}

#[derive(Debug, Clone)]
struct Rule {
    path: String,
    exact: bool,
    methods: Vec<String>,
}

impl ViewerPolicy {
    pub fn from_config(config: &PolicyConfig) -> Self {
        Self {
            default_deny: config.viewer_default_deny,
            allow: config.viewer_allow.iter().map(Rule::from).collect(),
            deny: config.viewer_deny.iter().map(Rule::from).collect(),
        }
    }

    pub fn allows(&self, method: &Method, path: &str) -> bool {
        if self.deny.iter().any(|rule| rule.matches(method, path)) {
            return false;
        }
        if self.allow.iter().any(|rule| rule.matches(method, path)) {
            return true;
        }
        !self.default_deny
    }
}

impl From<&PolicyRuleConfig> for Rule {
    fn from(value: &PolicyRuleConfig) -> Self {
        Self {
            path: value.path.clone(),
            exact: value.exact,
            methods: value.methods.iter().map(|method| method.trim().to_ascii_uppercase()).collect(),
        }
    }
}

impl Rule {
    fn matches(&self, method: &Method, path: &str) -> bool {
        let path_matches = if self.exact { path == self.path } else { path.starts_with(&self.path) };
        path_matches
            && (self.methods.is_empty()
                || self.methods.iter().any(|allowed| allowed == "*" || allowed == method.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Method;

    use super::ViewerPolicy;
    use crate::config::{PolicyConfig, PolicyRuleConfig};

    #[test]
    fn deny_wins_and_unknown_defaults_to_deny() {
        let policy = ViewerPolicy::from_config(&PolicyConfig {
            viewer_default_deny: true,
            viewer_allow: vec![PolicyRuleConfig {
                path: "/dbx/api/ai/".to_string(),
                exact: false,
                methods: vec!["POST".to_string()],
            }],
            viewer_deny: vec![PolicyRuleConfig {
                path: "/dbx/api/ai/configs".to_string(),
                exact: true,
                methods: vec!["POST".to_string()],
            }],
        });
        assert!(policy.allows(&Method::POST, "/dbx/api/ai/agent-stream"));
        assert!(!policy.allows(&Method::POST, "/dbx/api/ai/configs"));
        assert!(!policy.allows(&Method::GET, "/dbx/api/new-route"));
    }
}
