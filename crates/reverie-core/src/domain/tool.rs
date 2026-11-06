//! Tool metadata: descriptors exposed to the model, the dynamic-tool registry
//! rows, and the normalized execution result.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::time;

/// Interpreter used by a self-forged tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interpreter {
    Python3,
    Node,
    Bash,
}

impl Interpreter {
    pub fn as_str(self) -> &'static str {
        match self {
            Interpreter::Python3 => "python3",
            Interpreter::Node => "node",
            Interpreter::Bash => "bash",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "python3" | "python" | "py" => Ok(Interpreter::Python3),
            "node" | "nodejs" | "js" | "javascript" => Ok(Interpreter::Node),
            "bash" | "sh" | "shell" => Ok(Interpreter::Bash),
            other => Err(CoreError::InvalidEnum { kind: "Interpreter", value: other.to_string() }),
        }
    }

    /// Candidate executable names, in resolution order.
    pub fn executables(self) -> &'static [&'static str] {
        match self {
            Interpreter::Python3 => &["python3", "python"],
            Interpreter::Node => &["node"],
            Interpreter::Bash => &["bash", "sh"],
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Interpreter::Python3 => "py",
            Interpreter::Node => "mjs",
            Interpreter::Bash => "sh",
        }
    }
}

/// A callable tool as the UI and the model see it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub is_dynamic: bool,
    pub interpreter: Option<String>,
    pub usage_count: i64,
    pub created_by_agent_id: Option<String>,
}

impl ToolDescriptor {
    pub fn builtin(name: impl Into<String>, description: impl Into<String>, parameters: serde_json::Value) -> Self {
        ToolDescriptor {
            name: name.into(),
            description: description.into(),
            parameters,
            is_dynamic: false,
            interpreter: None,
            usage_count: 0,
            created_by_agent_id: None,
        }
    }
}

/// A persisted self-forged tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicToolRecord {
    pub id: String,
    pub tool_name: String,
    pub description: String,
    pub parameters_json_schema: String,
    pub interpreter: Interpreter,
    pub script_path: String,
    pub created_by_agent_id: Option<String>,
    pub is_verified: bool,
    pub usage_count: i64,
    pub created_at: DateTime<Utc>,
}

impl DynamicToolRecord {
    pub fn new(
        tool_name: impl Into<String>,
        description: impl Into<String>,
        parameters_json_schema: impl Into<String>,
        interpreter: Interpreter,
        script_path: impl Into<String>,
        created_by_agent_id: Option<String>,
    ) -> Self {
        DynamicToolRecord {
            id: uuid::Uuid::new_v4().to_string(),
            tool_name: tool_name.into(),
            description: description.into(),
            parameters_json_schema: parameters_json_schema.into(),
            interpreter,
            script_path: script_path.into(),
            created_by_agent_id,
            is_verified: false,
            usage_count: 0,
            created_at: time::now(),
        }
    }

    /// Parse the stored JSON schema, falling back to a permissive object schema.
    pub fn parameters(&self) -> serde_json::Value {
        serde_json::from_str(&self.parameters_json_schema)
            .unwrap_or_else(|_| serde_json::json!({"type": "object", "properties": {}}))
    }

    pub fn to_descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: self.tool_name.clone(),
            description: self.description.clone(),
            parameters: self.parameters(),
            is_dynamic: true,
            interpreter: Some(self.interpreter.as_str().to_string()),
            usage_count: self.usage_count,
            created_by_agent_id: self.created_by_agent_id.clone(),
        }
    }
}

/// Normalized tool execution result returned to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl ToolOutcome {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolOutcome { content: content.into(), is_error: false, metadata: serde_json::Value::Null }
    }

    pub fn ok_with(content: impl Into<String>, metadata: serde_json::Value) -> Self {
        ToolOutcome { content: content.into(), is_error: false, metadata }
    }

    pub fn error(content: impl Into<String>) -> Self {
        ToolOutcome { content: content.into(), is_error: true, metadata: serde_json::Value::Null }
    }

    /// Render for the tool-result message. Errors are prefixed so the model can
    /// distinguish them from successful output.
    pub fn to_message_content(&self) -> String {
        if self.is_error {
            format!("[tool_error]\n{}", self.content)
        } else {
            self.content.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpreter_parsing() {
        assert_eq!(Interpreter::parse("Python").unwrap(), Interpreter::Python3);
        assert_eq!(Interpreter::parse("js").unwrap(), Interpreter::Node);
        assert!(Interpreter::parse("ruby").is_err());
        assert_eq!(Interpreter::Bash.extension(), "sh");
    }

    #[test]
    fn descriptor_parses_stored_schema() {
        let record = DynamicToolRecord::new(
            "hash_file",
            "hash a file",
            "{\"type\":\"object\",\"properties\":{\"path\":{\"type\":\"string\"}}}",
            Interpreter::Python3,
            "/tmp/hash_file.py",
            Some("agent-1".into()),
        );
        let desc = record.to_descriptor();
        assert!(desc.is_dynamic);
        assert_eq!(desc.parameters["properties"]["path"]["type"], "string");
    }

    #[test]
    fn corrupt_schema_degrades() {
        let mut record = DynamicToolRecord::new("t", "d", "not-json", Interpreter::Bash, "/tmp/t.sh", None);
        record.parameters_json_schema = "{{{".into();
        assert_eq!(record.parameters()["type"], "object");
    }

    #[test]
    fn error_outcome_is_tagged() {
        let outcome = ToolOutcome::error("boom");
        assert!(outcome.to_message_content().starts_with("[tool_error]"));
        assert_eq!(ToolOutcome::ok("fine").to_message_content(), "fine");
    }
}
