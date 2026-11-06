//! Unified diff generation for workspace mutations.

use similar::{ChangeTag, TextDiff};

/// One file's change set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub diff: String,
    pub added: usize,
    pub removed: usize,
    pub is_new: bool,
}

impl FileDiff {
    /// One-line summary for logs and event previews.
    pub fn summary(&self) -> String {
        format!("{} (+{} −{})", self.path, self.added, self.removed)
    }

    /// Whether the file content actually changed.
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0
    }
}

/// Build a unified diff between two revisions of a file.
///
/// `path` is rendered as a relative repository-style path in the diff header.
pub fn unified_diff(old: &str, new: &str, path: &str) -> FileDiff {
    let diff = TextDiff::from_lines(old, new);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }

    let rendered = diff
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();

    FileDiff { path: path.to_string(), diff: rendered, added, removed, is_new: old.is_empty() && !new.is_empty() }
}

/// Render a compact multi-file diff, used in the inspector's diff tab.
pub fn render_bundle(diffs: &[FileDiff]) -> String {
    if diffs.is_empty() {
        return "No file changes recorded for this agent.".to_string();
    }
    let mut out = String::new();
    for item in diffs {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&item.diff);
        if !item.diff.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Total added/removed line counts across a bundle.
pub fn totals(diffs: &[FileDiff]) -> (usize, usize) {
    diffs.iter().fold((0, 0), |(added, removed), item| (added + item.added, removed + item.removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_added_and_removed_lines() {
        let old = "line one\nline two\nline three\n";
        let new = "line one\nline 2\nline three\nline four\n";
        let diff = unified_diff(old, new, "src/lib.rs");
        assert_eq!(diff.added, 2);
        assert_eq!(diff.removed, 1);
        assert!(diff.diff.contains("--- a/src/lib.rs"));
        assert!(diff.diff.contains("+++ b/src/lib.rs"));
        assert!(diff.diff.contains("+line four"));
    }

    #[test]
    fn identical_content_is_empty() {
        let diff = unified_diff("same\n", "same\n", "a.txt");
        assert!(diff.is_empty());
        assert_eq!(diff.added, 0);
    }

    #[test]
    fn new_file_is_flagged() {
        let diff = unified_diff("", "hello\n", "new.txt");
        assert!(diff.is_new);
        assert_eq!(diff.added, 1);
    }

    #[test]
    fn bundle_renders_all_files() {
        let a = unified_diff("a\n", "b\n", "one.txt");
        let b = unified_diff("c\n", "d\n", "two.txt");
        let bundle = render_bundle(&[a, b]);
        assert!(bundle.contains("one.txt"));
        assert!(bundle.contains("two.txt"));
        assert_eq!(totals(&[unified_diff("a\n", "b\n", "x"), unified_diff("c\n", "d\n", "y")]), (2, 2));
    }

    #[test]
    fn empty_bundle_has_message() {
        assert!(render_bundle(&[]).contains("No file changes"));
    }

    #[test]
    fn summary_is_compact() {
        let diff = unified_diff("a\n", "b\nc\n", "f.txt");
        assert!(diff.summary().starts_with("f.txt (+2"));
    }
}
