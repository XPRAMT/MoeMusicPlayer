//! A [`RemoteFs`] over a local folder: the scanner and the readers can be tested against real files without a
//! network, and the same trait object is what the app hands out for a remote account.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::fs::{RemoteEntry, RemoteError, RemoteFile, RemoteFs};

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
