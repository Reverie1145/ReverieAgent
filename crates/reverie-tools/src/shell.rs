//! Shell command execution with hard timeouts.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use reverie_core::domain::ToolOutcome;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::args::{opt_i64_clamped, opt_str, req_str};
use crate::context::ToolExecutionContext;
use crate::error::{ToolError, ToolResult};
use crate::registry::{AgentTool, ToolCategory};

/// Default command timeout: two minutes.
const DEFAULT_TIMEOUT_MS: i64 = 120_000;
/// Hard ceiling: ten minutes.
const MAX_TIMEOUT_MS: i64 = 600_000;

/// Run a shell command inside the agent's workspace.
pub struct ShellExecuteTool;

/// Build the platform-appropriate shell invocation.
pub fn shell_command(script: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(script);
        command
    } else {
        let mut command = Command::new("sh");
        command.arg("-c").arg(script);
        command
    }
}

#[async_trait]
impl AgentTool for ShellExecuteTool {
    fn name(&self) -> &str {
        "shell_execute"
    }

    fn description(&self) -> &str {
        "Execute a shell command in the agent's workspace directory and capture stdout, stderr and \
         the exit code. Commands are killed after the timeout."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Command line to run"},
                "cwd": {"type": "string", "description": "Working directory, relative to the workspace"},
                "timeout_ms": {
                    "type": "integer",
                    "description": "Kill the command after this many milliseconds",
                    "default": DEFAULT_TIMEOUT_MS
                }
            },
            "required": ["command"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Shell
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let script = req_str(&args, "command")?;
        if script.trim().is_empty() {
            return Err(ToolError::invalid("command must not be empty"));
        }
        let timeout_ms = opt_i64_clamped(&args, "timeout_ms", DEFAULT_TIMEOUT_MS, 1_000, MAX_TIMEOUT_MS);
        let cwd = match opt_str(&args, "cwd") {
            Some(raw) => ctx.resolve_writable(&raw)?,
            None => ctx.workspace_dir.clone(),
        };
        if !cwd.exists() {
            std::fs::create_dir_all(&cwd)?;
        }

        ctx.emit_activity(format!("Running: {}", reverie_core::util::truncate_chars(script.trim(), 60)));

        let mut command = shell_command(&script);
        command
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = command.spawn()?;
        let timeout = Duration::from_millis(timeout_ms as u64);
        let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(err)) => return Err(ToolError::Io(err)),
            Err(_) => {
                return Ok(ToolOutcome::error(format!(
                    "command timed out after {timeout_ms} ms and was terminated"
                )))
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);

        let mut rendered = format!("exit_code: {code}\n");
        if !stdout.trim().is_empty() {
            rendered.push_str("\n--- stdout ---\n");
            rendered.push_str(stdout.trim_end());
            rendered.push('\n');
        }
        if !stderr.trim().is_empty() {
            rendered.push_str("\n--- stderr ---\n");
            rendered.push_str(stderr.trim_end());
            rendered.push('\n');
        }
        if stdout.trim().is_empty() && stderr.trim().is_empty() {
            rendered.push_str("(no output)\n");
        }

        let outcome = ToolOutcome::ok_with(
            ctx.cap_output(rendered),
            json!({"exit_code": code, "stdout_bytes": output.stdout.len(), "stderr_bytes": output.stderr.len()}),
        );
        if code != 0 {
            return Ok(ToolOutcome {
                content: outcome.content,
                is_error: true,
                metadata: outcome.metadata,
            });
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::events::NoopSink;
    use std::sync::Arc;

    async fn context(dir: &std::path::Path) -> ToolExecutionContext {
        let store = Arc::new(reverie_store::Store::connect_in_memory().await.unwrap());
        ToolExecutionContext {
            session_id: "s".into(),
            agent_id: "a".into(),
            role_name: "tester".into(),
            workspace_dir: dir.to_path_buf(),
            store,
            vfs: Arc::new(crate::vfs::VfsLockManager::new()),
            sink: Arc::new(NoopSink),
            http: reqwest::Client::new(),
            search_config: reverie_core::domain::SearchEngineConfig::default(),
            web_search_enabled: true,
            registry: Arc::new(crate::registry::GlobalToolRegistry::new()),
            orchestration: None,
            max_output_chars: 20_000,
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("reverie-shell-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn successful_command_captures_stdout() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        let outcome = ShellExecuteTool.execute(&ctx, json!({"command": "echo reverie"})).await.unwrap();
        assert!(!outcome.is_error);
        assert!(outcome.content.contains("reverie"));
        assert_eq!(outcome.metadata["exit_code"], 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn failing_command_is_flagged() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        let command = if cfg!(windows) { "exit /b 3" } else { "exit 3" };
        let outcome = ShellExecuteTool.execute(&ctx, json!({"command": command})).await.unwrap();
        assert!(outcome.is_error);
        assert_eq!(outcome.metadata["exit_code"], 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn timeout_is_enforced() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        let command = if cfg!(windows) { "ping -n 6 127.0.0.1 > nul" } else { "sleep 5" };
        let outcome = ShellExecuteTool
            .execute(&ctx, json!({"command": command, "timeout_ms": 1000}))
            .await
            .unwrap();
        assert!(outcome.is_error);
        assert!(outcome.content.contains("timed out"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn empty_command_is_rejected() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        assert!(ShellExecuteTool.execute(&ctx, json!({"command": "  "})).await.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
