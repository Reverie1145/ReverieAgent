//! The Environment Warden: workspace provisioning, git worktree isolation and
//! branch merging.

use std::path::{Path, PathBuf};

use reverie_core::domain::{IsolationStrategy, Session};
use reverie_core::util::slugify;
use tokio::process::Command;

/// Where an agent will work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedWorkspace {
    pub dir: PathBuf,
    pub branch: Option<String>,
    /// True when the requested isolation could not be honoured.
    pub degraded: bool,
}

/// Result of merging an agent branch back into the session workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Clean,
    AlreadyUpToDate,
    Conflict { files: Vec<String> },
    Failed(String),
}

impl MergeOutcome {
    pub fn is_clean(&self) -> bool {
        matches!(self, MergeOutcome::Clean | MergeOutcome::AlreadyUpToDate)
    }

    pub fn describe(&self) -> String {
        match self {
            MergeOutcome::Clean => "merged cleanly".to_string(),
            MergeOutcome::AlreadyUpToDate => "branch was already up to date".to_string(),
            MergeOutcome::Conflict { files } => format!("conflicts in: {}", files.join(", ")),
            MergeOutcome::Failed(err) => format!("merge failed: {err}"),
        }
    }
}

/// Stateless policy engine for workspace isolation.
pub struct EnvironmentWardenAgent;

async fn run_git(cwd: &Path, args: &[&str]) -> Result<(i32, String, String), std::io::Error> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .output()
        .await?;
    Ok((
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    ))
}

impl EnvironmentWardenAgent {
    /// Whether a directory is inside a git work tree.
    pub async fn is_git_repo(path: &Path) -> bool {
        if !path.exists() {
            return false;
        }
        matches!(run_git(path, &["rev-parse", "--is-inside-work-tree"]).await, Ok((0, out, _)) if out.trim() == "true")
    }

    /// Branch name for an agent.
    pub fn branch_name(role_name: &str, agent_id: &str) -> String {
        let slug = slugify(role_name, 32);
        let short = agent_id.chars().take(8).collect::<String>();
        format!("reverie/{slug}-{short}")
    }

    /// Provision the directory an agent will work in.
    pub async fn provision(
        session: &Session,
        agent_id: &str,
        role_name: &str,
        isolation: IsolationStrategy,
    ) -> ProvisionedWorkspace {
        let root = PathBuf::from(&session.workspace_path);
        let _ = std::fs::create_dir_all(&root);

        match isolation {
            IsolationStrategy::SharedVfsLock => {
                ProvisionedWorkspace { dir: root, branch: None, degraded: false }
            }
            IsolationStrategy::SandboxDir => {
                let dir = root.join(reverie_core::RUNTIME_DIR).join("sandboxes").join(agent_id);
                let degraded = std::fs::create_dir_all(&dir).is_err();
                ProvisionedWorkspace {
                    dir: if degraded { root } else { dir },
                    branch: None,
                    degraded,
                }
            }
            IsolationStrategy::GitWorktree => {
                if !Self::is_git_repo(&root).await {
                    return ProvisionedWorkspace { dir: root, branch: None, degraded: true };
                }
                let branch = Self::branch_name(role_name, agent_id);
                let dir = root.join(reverie_core::RUNTIME_DIR).join("worktrees").join(agent_id);
                if let Some(parent) = dir.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let dir_arg = dir.to_string_lossy().to_string();
                let result = run_git(&root, &["worktree", "add", "-b", &branch, &dir_arg, "HEAD"]).await;
                match result {
                    Ok((0, _, _)) => {
                        ProvisionedWorkspace { dir, branch: Some(branch), degraded: false }
                    }
                    _ => ProvisionedWorkspace { dir: root, branch: None, degraded: true },
                }
            }
        }
    }

    /// Merge an agent branch into the current branch of `workspace`.
    pub async fn merge_branch(workspace: &Path, branch: &str) -> MergeOutcome {
        match run_git(workspace, &["merge", "--no-ff", "--no-edit", branch]).await {
            Ok((0, stdout, _)) => {
                if stdout.contains("Already up to date") || stdout.contains("Already up-to-date") {
                    MergeOutcome::AlreadyUpToDate
                } else {
                    MergeOutcome::Clean
                }
            }
            Ok((_, _, stderr)) => {
                let files = Self::conflicted_files(workspace).await;
                if files.is_empty() {
                    MergeOutcome::Failed(stderr.trim().to_string())
                } else {
                    MergeOutcome::Conflict { files }
                }
            }
            Err(err) => MergeOutcome::Failed(err.to_string()),
        }
    }

    /// Files currently in a conflicted state.
    pub async fn conflicted_files(workspace: &Path) -> Vec<String> {
        match run_git(workspace, &["diff", "--name-only", "--diff-filter=U"]).await {
            Ok((0, stdout, _)) => stdout
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Abort a merge that cannot be resolved automatically.
    pub async fn abort_merge(workspace: &Path) -> bool {
        matches!(run_git(workspace, &["merge", "--abort"]).await, Ok((0, _, _)))
    }

    /// Remove an agent worktree after its branch has been merged.
    pub async fn remove_worktree(workspace: &Path, dir: &Path) -> bool {
        let dir_arg = dir.to_string_lossy().to_string();
        matches!(run_git(workspace, &["worktree", "remove", "--force", &dir_arg]).await, Ok((0, _, _)))
    }

    /// Current branch name of a repository.
    pub async fn current_branch(workspace: &Path) -> Option<String> {
        match run_git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
            Ok((0, stdout, _)) => {
                let name = stdout.trim().to_string();
                if name.is_empty() {
                    None
                } else {
                    Some(name)
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_are_slugged_and_unique() {
        let branch = EnvironmentWardenAgent::branch_name("Auth Module Dev", "0123456789abcdef");
        assert_eq!(branch, "reverie/auth-module-dev-01234567");
    }

    #[test]
    fn merge_outcome_classification() {
        assert!(MergeOutcome::Clean.is_clean());
        assert!(MergeOutcome::AlreadyUpToDate.is_clean());
        assert!(!MergeOutcome::Conflict { files: vec!["a".into()] }.is_clean());
        assert!(MergeOutcome::Conflict { files: vec!["a.rs".into()] }.describe().contains("a.rs"));
    }

    #[tokio::test]
    async fn non_git_directory_degrades_to_shared() {
        let dir = std::env::temp_dir().join(format!("reverie-warden-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let session = Session::create("t", dir.to_string_lossy(), "o", "auto", true);

        let provisioned = EnvironmentWardenAgent::provision(
            &session,
            "agent-1",
            "Coder",
            IsolationStrategy::GitWorktree,
        )
        .await;
        assert!(provisioned.degraded);
        assert_eq!(provisioned.dir, dir);
        assert!(provisioned.branch.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sandbox_dirs_are_created() {
        let dir = std::env::temp_dir().join(format!("reverie-warden-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let session = Session::create("t", dir.to_string_lossy(), "o", "auto", true);
        let provisioned =
            EnvironmentWardenAgent::provision(&session, "agent-2", "Coder", IsolationStrategy::SandboxDir).await;
        assert!(!provisioned.degraded);
        assert!(provisioned.dir.ends_with("agent-2"));
        assert!(provisioned.dir.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn shared_mode_uses_the_workspace() {
        let dir = std::env::temp_dir().join(format!("reverie-warden-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let session = Session::create("t", dir.to_string_lossy(), "o", "auto", true);
        let provisioned =
            EnvironmentWardenAgent::provision(&session, "a", "Coder", IsolationStrategy::SharedVfsLock).await;
        assert_eq!(provisioned.dir, dir);
        assert!(!provisioned.degraded);
        std::fs::remove_dir_all(&dir).ok();
    }
}
