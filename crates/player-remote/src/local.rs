//! A [`RemoteFs`] over a local folder: the scanner and the readers can be tested against real files without a
//! network, and the same trait object is what the app hands out for a remote account.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::fs::{replacement_temp_path, FileStamp, RemoteEntry, RemoteError, RemoteFile, RemoteFs};

pub struct DirFs {
    root: PathBuf,
}

impl DirFs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, RemoteError> {
        let mut out = self.root.clone();
        for piece in path.split(['/', '\\']) {
            match piece {
                "" | "." => {}
                ".." => return Err(RemoteError::NotFound),
                other => out.push(other),
            }
        }
        Ok(out)
    }
}

fn entry_of(name: String, path: &Path) -> Result<RemoteEntry, RemoteError> {
    let metadata = std::fs::metadata(path).map_err(map_io)?;
    Ok(RemoteEntry {
        name,
        is_dir: metadata.is_dir(),
        size: if metadata.is_dir() { 0 } else { metadata.len() },
        modified_ms: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as i64),
        created_utc_100ns: None,
    })
}

fn map_io(error: std::io::Error) -> RemoteError {
    match error.kind() {
        std::io::ErrorKind::NotFound => RemoteError::NotFound,
        std::io::ErrorKind::PermissionDenied => RemoteError::Auth,
        _ => RemoteError::Network(error.to_string()),
    }
}

impl RemoteFs for DirFs {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        let path = self.resolve(dir)?;
        let mut entries = Vec::new();
        for item in std::fs::read_dir(&path).map_err(map_io)? {
            let item = item.map_err(map_io)?;
            let name = item.file_name().to_string_lossy().into_owned();
            entries.push(entry_of(name, &item.path())?);
        }
        Ok(entries)
    }

    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        let resolved = self.resolve(path)?;
        let name = path
            .rsplit(['/', '\\'])
            .find(|piece| !piece.is_empty())
            .unwrap_or("")
            .to_owned();
        entry_of(name, &resolved)
    }

    fn replace_file(
        &self,
        path: &str,
        source: &mut dyn Read,
        len: u64,
        expected: Option<FileStamp>,
    ) -> Result<RemoteEntry, RemoteError> {
        use std::io::Write;

        let target = self.resolve(path)?;
        if let Some(expected) = expected {
            let now = self.stat(path)?;
            if FileStamp::of(&now) != expected {
                return Err(RemoteError::Changed);
            }
        }
        let temp = self.resolve(&replacement_temp_path(path))?;
        let result = (|| {
            let mut out = File::create(&temp).map_err(map_io)?;
            let copied = std::io::copy(&mut source.take(len), &mut out).map_err(map_io)?;
            out.flush().map_err(map_io)?;
            drop(out);
            if copied != len {
                return Err(RemoteError::Protocol(format!("only {copied} of {len} bytes were sent")));
            }
            std::fs::rename(&temp, &target).map_err(map_io)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result?;
        self.stat(path)
    }

    fn remove_file(&self, path: &str) -> Result<(), RemoteError> {
        std::fs::remove_file(self.resolve(path)?).map_err(map_io)
    }

    fn rename_file(&self, from: &str, to: &str) -> Result<(), RemoteError> {
        let (from, to) = (self.resolve(from)?, self.resolve(to)?);
        // A rename never replaces on every platform (Windows refuses); keep that contract everywhere.
        if to.exists() {
            return Err(RemoteError::Protocol("the destination exists".to_owned()));
        }
        std::fs::rename(from, to).map_err(map_io)
    }

    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        let file = File::open(self.resolve(path)?).map_err(map_io)?;
        let len = file.metadata().map_err(map_io)?.len();
        Ok(Box::new(DirFile { file, len }))
    }
}

struct DirFile {
    file: File,
    len: u64,
}

impl RemoteFile for DirFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        self.file.seek(SeekFrom::Start(offset)).map_err(map_io)?;
        self.file.read(buf).map_err(map_io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moe-dirfs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        dir
    }

    #[test]
    fn a_file_is_replaced_through_a_temporary_file_and_nothing_is_left_behind() {
        let dir = folder("replace");
        std::fs::write(dir.join("a/song.mp3"), b"old bytes").unwrap();
        let fs = DirFs::new(&dir);
        let before = FileStamp::of(&fs.stat("a/song.mp3").unwrap());
        let new = vec![7_u8; 3 * 1024 * 1024 + 5];
        let entry = fs
            .replace_file("a/song.mp3", &mut new.as_slice(), new.len() as u64, Some(before))
            .unwrap();
        assert_eq!(entry.size, new.len() as u64);
        assert_eq!(std::fs::read(dir.join("a/song.mp3")).unwrap(), new);
        let names: Vec<_> = std::fs::read_dir(dir.join("a")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("song.mp3")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_changed_since_it_was_read_is_not_overwritten() {
        let dir = folder("changed");
        std::fs::write(dir.join("a/song.mp3"), b"read this").unwrap();
        let fs = DirFs::new(&dir);
        let before = FileStamp::of(&fs.stat("a/song.mp3").unwrap());
        // Someone else changes it (a different size).
        std::fs::write(dir.join("a/song.mp3"), b"somebody else wrote more than before").unwrap();
        let result = fs.replace_file("a/song.mp3", &mut &b"mine"[..], 4, Some(before));
        assert_eq!(result, Err(RemoteError::Changed));
        assert_eq!(std::fs::read(dir.join("a/song.mp3")).unwrap(), b"somebody else wrote more than before");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_short_source_leaves_the_original_and_no_temporary_file() {
        let dir = folder("short");
        std::fs::write(dir.join("a/song.mp3"), b"original").unwrap();
        let fs = DirFs::new(&dir);
        // 10 bytes promised, 4 delivered.
        let result = fs.replace_file("a/song.mp3", &mut &b"four"[..], 10, None);
        assert!(matches!(result, Err(RemoteError::Protocol(_))));
        assert_eq!(std::fs::read(dir.join("a/song.mp3")).unwrap(), b"original");
        assert!(!dir.join("a/.moetmp-song.mp3").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
