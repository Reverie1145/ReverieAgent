//! Virtual file locks.
//!
//! Agents run concurrently inside one workspace. Rather than serialising the
//! whole filesystem, each path gets its own async mutex so two agents editing
//! unrelated files never block each other, while concurrent writes to the same
//! file are strictly ordered.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Per-path asynchronous locks.
#[derive(Debug, Default)]
pub struct VfsLockManager {
    locks: DashMap<PathBuf, Arc<Mutex<()>>>,
}

impl VfsLockManager {
    pub fn new() -> Self {
        VfsLockManager { locks: DashMap::new() }
    }

    /// Normalize a path so `a/./b` and `a/b` share a lock.
    pub fn normalize(path: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    /// Acquire the lock for a path, waiting if another writer holds it.
    pub async fn lock(&self, path: &Path) -> OwnedMutexGuard<()> {
        let key = Self::normalize(path);
        let handle = self
            .locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        handle.lock_owned().await
    }

    /// Try to acquire without waiting.
    pub fn try_lock(&self, path: &Path) -> Option<OwnedMutexGuard<()>> {
        let key = Self::normalize(path);
        let handle = self
            .locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        handle.try_lock_owned().ok()
    }

    /// Whether a path is currently locked by someone.
    pub fn is_locked(&self, path: &Path) -> bool {
        let key = Self::normalize(path);
        match self.locks.get(&key) {
            Some(handle) => handle.try_lock().is_err(),
            None => false,
        }
    }

    /// Number of tracked paths.
    pub fn tracked(&self) -> usize {
        self.locks.len()
    }

    /// Drop lock entries nobody holds. Called after a session finishes.
    pub fn prune(&self) {
        self.locks.retain(|_, handle| Arc::strong_count(handle) > 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_path_serialises() {
        let manager = Arc::new(VfsLockManager::new());
        let guard = manager.lock(Path::new("/tmp/a.rs")).await;
        assert!(manager.try_lock(Path::new("/tmp/a.rs")).is_none());
        assert!(manager.try_lock(Path::new("/tmp/b.rs")).is_some());
        drop(guard);
        assert!(manager.try_lock(Path::new("/tmp/a.rs")).is_some());
    }

    #[tokio::test]
    async fn normalization_collapses_current_dir() {
        assert_eq!(
            VfsLockManager::normalize(Path::new("/tmp/./a/./b.rs")),
            PathBuf::from("/tmp/a/b.rs")
        );
    }

    #[tokio::test]
    async fn waiting_acquires_after_release() {
        let manager = Arc::new(VfsLockManager::new());
        let guard = manager.lock(Path::new("x")).await;
        let manager_clone = manager.clone();
        let waiter = tokio::spawn(async move {
            let _guard = manager_clone.lock(Path::new("x")).await;
            true
        });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(!waiter.is_finished());
        drop(guard);
        assert!(waiter.await.unwrap());
    }

    #[tokio::test]
    async fn prune_drops_idle_entries() {
        let manager = VfsLockManager::new();
        {
            let _guard = manager.lock(Path::new("a")).await;
        }
        assert_eq!(manager.tracked(), 1);
        manager.prune();
        assert_eq!(manager.tracked(), 0);
    }
}
