//! Workspace watching.
//!
//! Agents are not the only writers: a human may edit a file in their editor
//! while the swarm is running. This watcher turns those external changes into a
//! single debounced event so the UI can stay honest about what is on disk.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use reverie_core::events::{EventSink, SystemEvent};

/// How long to wait for a burst of filesystem events to settle.
const DEBOUNCE: Duration = Duration::from_millis(900);
/// Cap on how many paths one event carries.
const MAX_PATHS: usize = 20;
/// Directories that never produce useful signals (and change constantly).
const IGNORED_DIRS: [&str; 5] = [".reverie", ".git", "target", "node_modules", "dist"];

/// Whether a changed path should be reported.
pub fn is_relevant(root: &Path, path: &Path) -> bool {
    if !path.starts_with(root) {
        return false;
    }
    let relative = path.strip_prefix(root).unwrap_or(path);
    !relative.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            IGNORED_DIRS.iter().any(|ignored| name.eq_ignore_ascii_case(ignored))
        }
        _ => false,
    })
}

/// A live recursive watch over one workspace.
pub struct WorkspaceWatcher {
    /// Kept alive for as long as we want events; dropping it disconnects the
    /// channel and stops the worker thread.
    _watcher: notify::RecommendedWatcher,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl WorkspaceWatcher {
    /// Start watching `workspace`. Returns `None` when the platform refuses to
    /// set up a watch; callers treat that as "no watcher" rather than an error.
    pub fn start(workspace: &Path, session_id: String, sink: Arc<dyn EventSink>) -> Option<Self> {
        if !workspace.exists() {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            let _ = tx.send(result);
        })
        .ok()?;
        watcher.watch(workspace, RecursiveMode::Recursive).ok()?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let root = workspace.to_path_buf();

        let worker = std::thread::Builder::new()
            .name(format!(
                "reverie-watch-{}",
                reverie_core::util::char_prefix(&session_id, 8)
            ))
            .spawn(move || {
                let mut pending: Vec<PathBuf> = Vec::new();
                loop {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    match rx.recv_timeout(DEBOUNCE) {
                        Ok(Ok(event)) => {
                            for path in event.paths {
                                if is_relevant(&root, &path) && !pending.contains(&path) {
                                    pending.push(path);
                                }
                            }
                        }
                        // Watch errors (a file removed mid-scan) are noise.
                        Ok(Err(_)) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            flush(&mut pending, &root, &session_id, sink.as_ref());
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                flush(&mut pending, &root, &session_id, sink.as_ref());
            })
            .ok()?;

        Some(WorkspaceWatcher { _watcher: watcher, stop, worker: Some(worker) })
    }

    /// Stop the worker and release the OS watch.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for WorkspaceWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn flush(pending: &mut Vec<PathBuf>, root: &Path, session_id: &str, sink: &dyn EventSink) {
    if pending.is_empty() {
        return;
    }
    let total = pending.len();
    let paths: Vec<String> = pending
        .drain(..)
        .take(MAX_PATHS)
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    if total > paths.len() {
        tracing::debug!(total, reported = paths.len(), "workspace change burst truncated");
    }
    sink.emit(SystemEvent::WorkspaceFileChanged { session_id: session_id.to_string(), paths });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_internal_directories() {
        let root = Path::new("F:/ws");
        assert!(is_relevant(root, Path::new("F:/ws/src/main.rs")));
        assert!(!is_relevant(root, Path::new("F:/ws/.reverie/attachments/a.png")));
        assert!(!is_relevant(root, Path::new("F:/ws/target/debug/x.exe")));
        assert!(!is_relevant(root, Path::new("F:/ws/node_modules/react/index.js")));
        assert!(!is_relevant(root, Path::new("F:/ws/.git/HEAD")));
    }

    #[test]
    fn ignores_paths_outside_the_workspace() {
        let root = Path::new("F:/ws");
        assert!(!is_relevant(root, Path::new("F:/other/file.rs")));
    }

    #[test]
    fn ignore_matching_is_case_insensitive() {
        let root = Path::new("F:/ws");
        assert!(!is_relevant(root, Path::new("F:/ws/Target/debug/a")));
        assert!(!is_relevant(root, Path::new("F:/ws/.GIT/HEAD")));
    }
}
