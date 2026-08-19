use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::agent_events::{ToolCall, ToolDefinition, ToolResult};

use super::AgentFileService;

#[async_trait]
pub trait AgentFunctionExtension: Send + Sync {
    fn definitions(&self) -> Vec<ToolDefinition>;
    fn handles(&self, name: &str) -> bool;
    async fn execute(&self, call: &ToolCall) -> ToolResult;
}

#[derive(Clone)]
pub struct AgentFunctionRegistry {
    extensions: Arc<Vec<Arc<dyn AgentFunctionExtension>>>,
}

impl Default for AgentFunctionRegistry {
    fn default() -> Self {
        Self { extensions: Arc::new(vec![Arc::new(AgentFileService::default())]) }
    }
}

impl AgentFunctionRegistry {
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut names = HashSet::new();
        let mut definitions = Vec::new();
        for extension in self.extensions.iter() {
            for definition in extension.definitions() {
                assert!(names.insert(definition.name), "duplicate agent extension tool name: {}", definition.name);
                definitions.push(definition);
            }
        }
        definitions
    }

    pub fn handles(&self, name: &str) -> bool {
        self.extensions.iter().any(|extension| extension.handles(name))
    }

    pub async fn execute(&self, call: &ToolCall) -> Option<ToolResult> {
        for extension in self.extensions.iter() {
            if extension.handles(&call.name) {
                return Some(extension.execute(call).await);
            }
        }
        None
    }
}

#[async_trait]
impl AgentFunctionExtension for AgentFileService {
    fn definitions(&self) -> Vec<ToolDefinition> {
        AgentFileService::definitions(self)
    }

    fn handles(&self, name: &str) -> bool {
        AgentFileService::handles(self, name)
    }

    async fn execute(&self, call: &ToolCall) -> ToolResult {
        AgentFileService::execute(self, call).await
    }
}
