//! Runtime self-tooling: agents write small scripts, they get sandbox-tested,
//! then hot-registered so every other agent can call them immediately.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use reverie_core::domain::{DynamicToolRecord, Interpreter, ToolOutcome};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::args::{opt_i64_clamped, opt_object, req_str};
use crate::context::ToolExecutionContext;
use crate::error::{ToolError, ToolResult};
use crate::registry::{AgentTool, ToolCategory};

/// Sandbox verification budget.
const VERIFY_TIMEOUT_MS: u64 = 15_000;
/// Budget for a normal invocation.
const RUN_TIMEOUT_MS: u64 = 60_000;
/// Cap on captured stdout/stderr.
const MAX_CAPTURE: usize = 200_000;

/// A tool forged at runtime by an agent.
#[derive(Debug)]
pub struct DynamicScriptTool {
    record: DynamicToolRecord,
    parameters: Value,
    usage: AtomicI64,
}

impl DynamicScriptTool {
    pub fn new(record: DynamicToolRecord) -> Self {
        let parameters = record.parameters();
        let usage = AtomicI64::new(record.usage_count);
        DynamicScriptTool { record, parameters, usage }
    }

    pub fn record(&self) -> &DynamicToolRecord {
        &self.record
    }

    /// Execute the script with `input` on stdin.
    async fn run(&self, input: &Value, cwd: &Path, timeout_ms: u64) -> ToolResult<(i32, String, String)> {
        let script = PathBuf::from(&self.record.script_path);
        if !script.exists() {
            return Err(ToolError::NotFound(format!("script {}", script.display())));
        }

        let payload = input.to_string();
        let mut missing: Option<std::io::Error> = None;

        for executable in self.record.interpreter.executables() {
            let mut command = Command::new(executable);
            command
                .arg(&script)
                .current_dir(cwd)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    missing = Some(err);
                    continue;
                }
                Err(err) => return Err(ToolError::Io(err)),
            };

            if let Some(mut stdin) = child.stdin.take() {
                // A script that ignores stdin may close the pipe early.
                let _ = stdin.write_all(payload.as_bytes()).await;
                let _ = stdin.shutdown().await;
            }

            let output = match tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await {
                Ok(Ok(output)) => output,
                Ok(Err(err)) => return Err(ToolError::Io(err)),
                Err(_) => {
                    return Err(ToolError::Timeout(timeout_ms));
                }
            };

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let code = output.status.code().unwrap_or(-1);
            return Ok((code, truncate(stdout), truncate(stderr)));
        }

        Err(ToolError::unavailable(format!(
            "no `{}` executable on PATH ({})",
            self.record.interpreter.as_str(),
            missing.map(|err| err.to_string()).unwrap_or_else(|| "not found".to_string())
        )))
    }
}

fn truncate(text: String) -> String {
    if text.len() <= MAX_CAPTURE {
        text
    } else {
        text.chars().take(MAX_CAPTURE).collect()
    }
}

#[async_trait]
impl AgentTool for DynamicScriptTool {
    fn name(&self) -> &str {
        &self.record.tool_name
    }

    fn description(&self) -> &str {
        &self.record.description
    }

    fn parameters(&self) -> Value {
        self.parameters.clone()
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Dynamic
    }

    fn is_dynamic(&self) -> bool {
        true
    }

    fn interpreter(&self) -> Option<String> {
        Some(self.record.interpreter.as_str().to_string())
    }

    fn usage_count(&self) -> i64 {
        self.usage.load(Ordering::Relaxed)
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        self.usage.fetch_add(1, Ordering::Relaxed);
        let _ = ctx.store.increment_tool_usage(&self.record.tool_name).await;

        ctx.emit_activity(format!("Running forged tool `{}`", self.record.tool_name));
        let (code, stdout, stderr) = self.run(&args, &ctx.workspace_dir, RUN_TIMEOUT_MS).await?;

        let mut rendered = format!("exit_code: {code}\n");
        if !stdout.trim().is_empty() {
            rendered.push_str(&stdout);
        }
        if !stderr.trim().is_empty() {
            rendered.push_str("\n--- stderr ---\n");
            rendered.push_str(&stderr);
        }

        let outcome = ToolOutcome::ok_with(
            ctx.cap_output(rendered),
            json!({"exit_code": code, "tool": self.record.tool_name}),
        );
        if code != 0 {
            return Ok(ToolOutcome { content: outcome.content, is_error: true, metadata: outcome.metadata });
        }
        Ok(outcome)
    }
}

fn valid_tool_name(name: &str) -> bool {
    let pattern = Regex::new(r"^[a-z][a-z0-9_]{2,48}$").expect("static regex");
    pattern.is_match(name)
}

/// Forge, verify and register a new tool.
pub struct ForgeDynamicToolTool;

#[async_trait]
impl AgentTool for ForgeDynamicToolTool {
    fn name(&self) -> &str {
        "forge_dynamic_tool"
    }

    fn description(&self) -> &str {
        "Create a new reusable tool at runtime. Write a self-contained python3/node/bash script that \
         reads a JSON object from stdin and prints its result to stdout. The script is executed once \
         with `test_input` as a sandbox verification; only verified tools are registered for the \
         whole swarm."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "snake_case identifier, 3-49 chars, starting with a letter"
                },
                "description": {"type": "string", "description": "What the tool does, for other agents"},
                "interpreter": {"type": "string", "enum": ["python3", "node", "bash"], "default": "python3"},
                "parameters_schema": {
                    "type": "object",
                    "description": "JSON Schema for the tool's arguments"
                },
                "script": {"type": "string", "description": "Full script source"},
                "test_input": {"type": "object", "description": "Sample arguments used for verification"},
                "timeout_ms": {"type": "integer", "description": "Verification timeout", "default": 15000}
            },
            "required": ["tool_name", "description", "interpreter", "parameters_schema", "script", "test_input"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let tool_name = req_str(&args, "tool_name")?;
        if !valid_tool_name(&tool_name) {
            return Err(ToolError::invalid(
                "tool_name must be snake_case: start with a letter, then letters/digits/underscores, 3-49 chars",
            ));
        }
        if let Some(existing) = ctx.registry.get(&tool_name) {
            if !existing.is_dynamic() {
                return Err(ToolError::Denied(format!(
                    "`{tool_name}` collides with a built-in tool; pick another name"
                )));
            }
        }

        let description = req_str(&args, "description")?;
        let interpreter = Interpreter::parse(&req_str(&args, "interpreter")?)?;
        let schema = opt_object(&args, "parameters_schema");
        let parameters_schema = if schema.is_empty() {
            json!({"type": "object", "properties": {}})
        } else {
            Value::Object(schema)
        };
        let script = req_str(&args, "script")?;
        if script.trim().is_empty() {
            return Err(ToolError::invalid("script must not be empty"));
        }
        let test_input = args.get("test_input").cloned().unwrap_or_else(|| json!({}));
        let timeout_ms = opt_i64_clamped(&args, "timeout_ms", VERIFY_TIMEOUT_MS as i64, 1_000, 120_000) as u64;

        let dir = ctx.custom_tools_dir();
        std::fs::create_dir_all(&dir)?;
        let script_path = dir.join(format!("{tool_name}.{}", interpreter.extension()));
        std::fs::write(&script_path, &script)?;

        let mut record = DynamicToolRecord::new(
            &tool_name,
            &description,
            parameters_schema.to_string(),
            interpreter,
            script_path.to_string_lossy().to_string(),
            Some(ctx.agent_id.clone()),
        );
        record.is_verified = false;

        ctx.emit_activity(format!("Verifying forged tool `{tool_name}`"));
        let candidate = DynamicScriptTool::new(record.clone());
        let verification = candidate.run(&test_input, &ctx.workspace_dir, timeout_ms).await;

        match verification {
            Ok((0, stdout, _)) => {
                record.is_verified = true;
                ctx.store.upsert_dynamic_tool(&record).await?;
                ctx.registry.register(Arc::new(DynamicScriptTool::new(record.clone())));
                ctx.log("info", format!("Registered forged tool `{tool_name}`"));
                Ok(ToolOutcome::ok_with(
                    format!(
                        "Tool `{tool_name}` verified and registered for all agents.\n\nVerification output:\n{stdout}"
                    ),
                    json!({"tool_name": tool_name, "script_path": record.script_path, "verified": true}),
                ))
            }
            Ok((code, stdout, stderr)) => {
                record.is_verified = false;
                ctx.store.upsert_dynamic_tool(&record).await?;
                Ok(ToolOutcome::error(format!(
                    "Verification failed with exit code {code}. The tool was saved but NOT registered.\n\
                     stdout:\n{stdout}\nstderr:\n{stderr}"
                )))
            }
            Err(err) => {
                record.is_verified = false;
                ctx.store.upsert_dynamic_tool(&record).await?;
                Ok(ToolOutcome::error(format!(
                    "Verification could not run: {err}. The script was saved at {} but NOT registered.",
                    record.script_path
                )))
            }
        }
    }
}

/// Load previously verified tools from the database at startup.
pub async fn load_verified_tools(store: &reverie_store::Store) -> Vec<Arc<DynamicScriptTool>> {
    match store.list_verified_dynamic_tools().await {
        Ok(records) => records.into_iter().map(|record| Arc::new(DynamicScriptTool::new(record))).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_validation() {
        assert!(valid_tool_name("count_lines"));
        assert!(valid_tool_name("abc"));
        assert!(!valid_tool_name("Bad-Name"));
        assert!(!valid_tool_name("ab"));
        assert!(!valid_tool_name("1abc"));
    }

    #[test]
    fn interpreter_extension_matches() {
        assert_eq!(Interpreter::Python3.extension(), "py");
        assert_eq!(Interpreter::Node.extension(), "mjs");
    }
}
