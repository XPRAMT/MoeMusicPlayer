//! Which font files must be reparsed (ADR 0042): the cache keeps each file's size and modification time.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A font file as seen on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStamp {
    pub path: String,
    pub size_bytes: u64,
    /// Modification time, UTC ms since the epoch (0 when unknown).
    pub modified_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// New files and files whose size or time changed.
    pub to_parse: Vec<FileStamp>,
    /// Cached files that are gone.
    pub removed: Vec<String>,
    /// Cached files that are still the same.
    pub unchanged: usize,
}

impl FileDiff {
    pub fn is_empty(&self) -> bool {
        self.to_parse.is_empty() && self.removed.is_empty()
    }
}

/// Compare the cached stamps with the files present now.
pub fn diff_files(cached: &[FileStamp], current: &[FileStamp]) -> FileDiff {
    let known: HashMap<&str, &FileStamp> = cached.iter().map(|s| (s.path.as_str(), s)).collect();
    let mut diff = FileDiff::default();
    for stamp in current {
        match known.get(stamp.path.as_str()) {
            Some(old) if *old == stamp => diff.unchanged += 1,
            _ => diff.to_parse.push(stamp.clone()),
        }
    }
    let present: std::collections::HashSet<&str> =
        current.iter().map(|s| s.path.as_str()).collect();
    diff.removed = cached
        .iter()
        .filter(|s| !present.contains(s.path.as_str()))
        .map(|s| s.path.clone())
        .collect();
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(path: &str, size: u64, modified: i64) -> FileStamp {
        FileStamp {
            path: path.to_owned(),
            size_bytes: size,
            modified_ms: modified,
        }
    }

    #[test]
    fn unchanged_changed_new_and_removed_files() {
        let cached = vec![
            stamp("a.ttf", 10, 1),
            stamp("b.ttf", 20, 2),
            stamp("c.ttc", 30, 3),
        ];
        let current = vec![
            stamp("a.ttf", 10, 1),
            stamp("b.ttf", 21, 2),
            stamp("c.ttc", 30, 4),
            stamp("d.otf", 5, 5),
        ];
        let diff = diff_files(&cached, &current);
        assert_eq!(diff.unchanged, 1);
        let parsed: Vec<_> = diff.to_parse.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(parsed, vec!["b.ttf", "c.ttc", "d.otf"]);
        assert!(diff.removed.is_empty());

        let diff = diff_files(&cached, &[stamp("a.ttf", 10, 1)]);
        assert_eq!(diff.removed, vec!["b.ttf".to_owned(), "c.ttc".to_owned()]);
        assert!(diff.to_parse.is_empty());
        assert!(!diff.is_empty());
        assert!(diff_files(&cached, &cached).is_empty());
        // Empty cache: everything is parsed.
        assert_eq!(diff_files(&[], &cached).to_parse.len(), 3);
    }
}
