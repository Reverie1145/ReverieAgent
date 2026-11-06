//! Filesystem primitives: read, write, patch and gitignore-aware search.

use std::path::Path;

use async_trait::async_trait;
use ignore::WalkBuilder;
use regex::Regex;
use reverie_core::domain::ToolOutcome;
use serde_json::{json, Value};

use crate::args::{opt_bool, opt_i64_clamped, opt_str, req_str};
use crate::context::ToolExecutionContext;
use crate::diff::unified_diff;
use crate::error::{ToolError, ToolResult};
use crate::registry::{AgentTool, ToolCategory};

/// Render a path relative to the workspace for display and diff headers.
pub fn display_path(ctx: &ToolExecutionContext, path: &Path) -> String {
    let relative = path.strip_prefix(&ctx.workspace_dir).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

/// Refuse to read files larger than this; models cannot use them anyway.
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
/// Default number of lines returned by a read.
const DEFAULT_READ_LINES: i64 = 400;

/// Read a workspace file with line-range pagination.
pub struct FsReadFileTool;

#[async_trait]
impl AgentTool for FsReadFileTool {
    fn name(&self) -> &str {
        "fs_read_file"
    }

    fn description(&self) -> &str {
        "Read a workspace file with 1-based line pagination. Returns numbered lines plus the total \
         line count so you can page through large files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path relative to the workspace, or absolute"},
                "start_line": {"type": "integer", "description": "First line to return (1-based)", "default": 1},
                "end_line": {"type": "integer", "description": "Last line to return (inclusive)"}
            },
            "required": ["path"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Filesystem
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let raw = req_str(&args, "path")?;
        let path = ctx.resolve(&raw);
        if !path.exists() {
            return Err(ToolError::NotFound(format!("{}", path.display())));
        }
        if !path.is_file() {
            return Err(ToolError::invalid(format!("{} is not a file", path.display())));
        }
        let metadata = std::fs::metadata(&path)?;
        if metadata.len() > MAX_READ_BYTES {
            return Err(ToolError::invalid(format!(
                "{} is {} bytes, above the {} byte read limit",
                display_path(ctx, &path),
                metadata.len(),
                MAX_READ_BYTES
            )));
        }

        let content = std::fs::read_to_string(&path).map_err(|err| {
            ToolError::other(format!("{} is not valid UTF-8 text: {err}", display_path(ctx, &path)))
        })?;

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let start = opt_i64_clamped(&args, "start_line", 1, 1, total.max(1) as i64) as usize;
        let default_end = (start as i64 + DEFAULT_READ_LINES - 1).min(total as i64).max(1);
        let end = opt_i64_clamped(&args, "end_line", default_end, 1, total.max(1) as i64) as usize;
        let end = end.max(start);
        let slice = &lines[(start - 1).min(total)..end.min(total)];

        let mut output = format!(
            "{} (lines {}-{} of {})\n",
            display_path(ctx, &path),
            start,
            end.min(total),
            total
        );
        for (offset, line) in slice.iter().enumerate() {
            output.push_str(&format!("{:>6}\t{}\n", start + offset, line));
        }
        if end < total {
            output.push_str(&format!(
                "\n[{} more lines; call again with start_line={}]\n",
                total - end,
                end + 1
            ));
        }

        Ok(ToolOutcome::ok_with(
            ctx.cap_output(output),
            json!({"path": display_path(ctx, &path), "total_lines": total, "start": start, "end": end.min(total)}),
        ))
    }
}

/// Create or overwrite a workspace file.
pub struct FsWriteFileTool;

#[async_trait]
impl AgentTool for FsWriteFileTool {
    fn name(&self) -> &str {
        "fs_write_file"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file inside the agent workspace. Protected by the virtual file lock \
         and emits a live unified diff to the UI."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path relative to the workspace"},
                "content": {"type": "string", "description": "Full file content"}
            },
            "required": ["path", "content"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Filesystem
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let raw = req_str(&args, "path")?;
        let content = req_str(&args, "content")?;
        let path = ctx.resolve_writable(&raw)?;

        let _guard = ctx.vfs.lock(&path).await;
        ctx.ensure_parent(&path)?;
        let previous = std::fs::read_to_string(&path).unwrap_or_default();
        std::fs::write(&path, &content)?;

        let label = display_path(ctx, &path);
        let diff = unified_diff(&previous, &content, &label);
        ctx.emit_diff(&diff);
        ctx.emit_activity(format!("Wrote {label}"));

        Ok(ToolOutcome::ok_with(
            format!("Wrote {label} (+{} −{})", diff.added, diff.removed),
            json!({"path": label, "added": diff.added, "removed": diff.removed, "created": diff.is_new}),
        ))
    }
}

/// Replace a targeted block inside an existing file.
pub struct FsEditPatchTool;

#[async_trait]
impl AgentTool for FsEditPatchTool {
    fn name(&self) -> &str {
        "fs_edit_patch"
    }

    fn description(&self) -> &str {
        "Replace an exact block of text inside an existing file. Prefer this over rewriting whole \
         files: it is cheaper, safer and produces a minimal diff. Fails loudly if the old text is \
         missing or ambiguous."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path relative to the workspace"},
                "old_string": {"type": "string", "description": "Exact text to replace, including indentation"},
                "new_string": {"type": "string", "description": "Replacement text"},
                "replace_all": {"type": "boolean", "description": "Replace every occurrence", "default": false}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Filesystem
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let raw = req_str(&args, "path")?;
        let old_string = req_str(&args, "old_string")?;
        let new_string = req_str(&args, "new_string")?;
        let replace_all = opt_bool(&args, "replace_all", false);

        if old_string.is_empty() {
            return Err(ToolError::invalid("old_string must not be empty"));
        }
        if old_string == new_string {
            return Err(ToolError::invalid("old_string and new_string are identical"));
        }

        let path = ctx.resolve_writable(&raw)?;
        let _guard = ctx.vfs.lock(&path).await;

        let previous = std::fs::read_to_string(&path)
            .map_err(|_| ToolError::NotFound(format!("{}", display_path(ctx, &path))))?;

        let occurrences = previous.matches(&old_string).count();
        if occurrences == 0 {
            return Err(ToolError::other(format!(
                "old_string not found in {}. Read the file again and copy the exact text.",
                display_path(ctx, &path)
            )));
        }
        if occurrences > 1 && !replace_all {
            return Err(ToolError::other(format!(
                "old_string appears {occurrences} times in {}; add more surrounding context or set \
                 replace_all=true",
                display_path(ctx, &path)
            )));
        }

        let updated = if replace_all {
            previous.replace(&old_string, &new_string)
        } else {
            previous.replacen(&old_string, &new_string, 1)
        };
        std::fs::write(&path, &updated)?;

        let label = display_path(ctx, &path);
        let diff = unified_diff(&previous, &updated, &label);
        ctx.emit_diff(&diff);
        ctx.emit_activity(format!("Edited {label}"));

        Ok(ToolOutcome::ok_with(
            format!("Patched {label} (+{} −{}, {occurrences} match(es))", diff.added, diff.removed),
            json!({"path": label, "occurrences": occurrences, "added": diff.added, "removed": diff.removed}),
        ))
    }
}

/// List a directory tree or search file contents with a regex.
pub struct FsListAndSearchTool;

fn build_walker(root: &Path, max_depth: Option<usize>, include_hidden: bool) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!include_hidden)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .max_depth(max_depth);
    builder
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

#[async_trait]
impl AgentTool for FsListAndSearchTool {
    fn name(&self) -> &str {
        "fs_list_and_search"
    }

    fn description(&self) -> &str {
        "Fast, .gitignore-aware directory listing or regex content search. Use mode=\"list\" to \
         explore the tree and mode=\"search\" to find code by pattern."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {"type": "string", "enum": ["list", "search"], "default": "list"},
                "path": {"type": "string", "description": "Directory or file to start from", "default": "."},
                "pattern": {"type": "string", "description": "Rust regex, required for search mode"},
                "max_results": {"type": "integer", "description": "Cap on entries or matches", "default": 200},
                "max_depth": {"type": "integer", "description": "Directory depth limit", "default": 8},
                "include_hidden": {"type": "boolean", "description": "Include dotfiles", "default": false}
            },
            "required": []
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Filesystem
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let mode = opt_str(&args, "mode").unwrap_or_else(|| "list".to_string());
        let root_raw = opt_str(&args, "path").unwrap_or_else(|| ".".to_string());
        let root = ctx.resolve(&root_raw);
        if !root.exists() {
            return Err(ToolError::NotFound(format!("{}", root.display())));
        }
        let max_results = opt_i64_clamped(&args, "max_results", 200, 1, 2000) as usize;
        let max_depth = opt_i64_clamped(&args, "max_depth", 8, 1, 40) as usize;
        let include_hidden = opt_bool(&args, "include_hidden", false);

        match mode.as_str() {
            "search" => {
                let pattern = req_str(&args, "pattern")?;
                let regex = Regex::new(&pattern)
                    .map_err(|err| ToolError::invalid(format!("invalid regex: {err}")))?;
                ctx.emit_activity(format!("Searching for /{}/", pattern));

                let walker = build_walker(&root, Some(max_depth), include_hidden);
                let mut output = String::new();
                let mut matches = 0usize;
                let mut scanned = 0usize;

                for entry in walker.build().flatten() {
                    if matches >= max_results {
                        break;
                    }
                    let path = entry.path();
                    if !entry.file_type().map(|kind| kind.is_file()).unwrap_or(false) {
                        continue;
                    }
                    scanned += 1;
                    if scanned > 20_000 {
                        break;
                    }
                    let Ok(bytes) = std::fs::read(path) else {
                        continue;
                    };
                    if looks_binary(&bytes) {
                        continue;
                    }
                    let Ok(text) = String::from_utf8(bytes) else {
                        continue;
                    };
                    let label = display_path(ctx, path);
                    for (index, line) in text.lines().enumerate() {
                        if matches >= max_results {
                            break;
                        }
                        if regex.is_match(line) {
                            matches += 1;
                            let trimmed = line.trim();
                            let rendered = reverie_core::util::truncate_chars(trimmed, 240);
                            output.push_str(&format!("{label}:{}: {rendered}\n", index + 1));
                        }
                    }
                }

                if matches == 0 {
                    output.push_str(&format!("No matches for /{pattern}/ under {root_raw}\n"));
                } else {
                    output.push_str(&format!("\n{matches} match(es) across {scanned} files scanned\n"));
                }
                Ok(ToolOutcome::ok_with(
                    ctx.cap_output(output),
                    json!({"mode": "search", "matches": matches, "files_scanned": scanned}),
                ))
            }
            _ => {
                let walker = build_walker(&root, Some(max_depth), include_hidden);
                let mut entries: Vec<String> = Vec::new();
                for entry in walker.build().flatten() {
                    if entries.len() >= max_results {
                        break;
                    }
                    let path = entry.path();
                    if path == root {
                        continue;
                    }
                    let label = display_path(ctx, path);
                    let marker = if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                        "/"
                    } else {
                        ""
                    };
                    entries.push(format!("{label}{marker}"));
                }
                entries.sort();

                let mut output = format!("{root_raw} ({} entries)\n", entries.len());
                for entry in &entries {
                    output.push_str(entry);
                    output.push('\n');
                }
                if entries.len() >= max_results {
                    output.push_str(&format!("\n[stopped at the {max_results} entry limit]\n"));
                }
                Ok(ToolOutcome::ok_with(
                    ctx.cap_output(output),
                    json!({"mode": "list", "entries": entries.len()}),
                ))
            }
        }
    }
}

/// Convenience helper used by other modules.
pub fn read_text(path: &Path) -> ToolResult<String> {
    Ok(std::fs::read_to_string(path)?)
}

/// All four filesystem tools.
pub fn fs_tools() -> Vec<std::sync::Arc<dyn AgentTool>> {
    vec![
        std::sync::Arc::new(FsReadFileTool),
        std::sync::Arc::new(FsWriteFileTool),
        std::sync::Arc::new(FsEditPatchTool),
        std::sync::Arc::new(FsListAndSearchTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::events::NoopSink;
    use std::path::PathBuf;
    use std::sync::Arc;

    async fn context(dir: &Path) -> ToolExecutionContext {
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
            max_output_chars: 50_000,
        }
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reverie-fs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn binary_detection() {
        assert!(looks_binary(&[0x89, 0x50, 0x00, 0x4E]));
        assert!(!looks_binary(b"plain text file"));
    }

    #[tokio::test]
    async fn write_then_read_roundtrip() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        let tool = FsWriteFileTool;
        tool.execute(&ctx, json!({"path": "src/lib.rs", "content": "fn a() {}\nfn b() {}\n"}))
            .await
            .unwrap();

        let outcome = FsReadFileTool
            .execute(&ctx, json!({"path": "src/lib.rs", "start_line": 2}))
            .await
            .unwrap();
        assert!(outcome.content.contains("fn b()"));
        assert!(!outcome.content.contains("fn a()"));
        assert_eq!(outcome.metadata["total_lines"], 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn write_outside_workspace_is_denied() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        let err = FsWriteFileTool
            .execute(&ctx, json!({"path": "../../etc/passwd", "content": "x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn edit_patch_replaces_exact_block() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        FsWriteFileTool
            .execute(&ctx, json!({"path": "a.txt", "content": "alpha\nbeta\ngamma\n"}))
            .await
            .unwrap();

        FsEditPatchTool
            .execute(&ctx, json!({"path": "a.txt", "old_string": "beta", "new_string": "BETA"}))
            .await
            .unwrap();

        let written = std::fs::read_to_string(dir.join("a.txt")).unwrap();
        assert_eq!(written, "alpha\nBETA\ngamma\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn edit_patch_rejects_ambiguous_and_missing_text() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        FsWriteFileTool
            .execute(&ctx, json!({"path": "b.txt", "content": "dup\ndup\n"}))
            .await
            .unwrap();

        assert!(FsEditPatchTool
            .execute(&ctx, json!({"path": "b.txt", "old_string": "dup", "new_string": "x"}))
            .await
            .is_err());
        assert!(FsEditPatchTool
            .execute(&ctx, json!({"path": "b.txt", "old_string": "missing", "new_string": "x"}))
            .await
            .is_err());
        assert!(FsEditPatchTool
            .execute(&ctx, json!({"path": "b.txt", "old_string": "dup", "new_string": "x", "replace_all": true}))
            .await
            .is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn list_and_search_finds_pattern() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        FsWriteFileTool
            .execute(&ctx, json!({"path": "src/main.rs", "content": "fn main() {\n    println!(\"hi\");\n}\n"}))
            .await
            .unwrap();

        let listing = FsListAndSearchTool
            .execute(&ctx, json!({"mode": "list", "path": "."}))
            .await
            .unwrap();
        assert!(listing.content.contains("src/main.rs"));

        let found = FsListAndSearchTool
            .execute(&ctx, json!({"mode": "search", "path": ".", "pattern": "println"}))
            .await
            .unwrap();
        assert!(found.content.contains("src/main.rs:2"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_file_reports_not_found() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        let err = FsReadFileTool.execute(&ctx, json!({"path": "nope.txt"})).await.unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
