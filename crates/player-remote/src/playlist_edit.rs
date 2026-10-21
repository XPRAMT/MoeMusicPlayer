//! Adding a song to, or removing it from, a playlist file that lives on a server (ADR 0026).
//!
//! The file is read fresh, only the lines for the song are touched (every other byte stays: encoding and byte-order
//! mark, line endings, header, comments, `#EXTINF` lines of other entries), the new entry is written in the style the
//! file's own entries use ([`PlaylistStyle`]), and the result replaces the original with the size and modified time
//! taken at the read, so a file somebody else changed meanwhile is refused ([`RemoteError::Changed`]) and stays as it is.

use std::io::Cursor;

use crate::fs::{FileStamp, RemoteError, RemoteFs};
use crate::playlist_style::PlaylistStyle;

/// A playlist file larger than this is not a playlist.
pub const MAX_PLAYLIST_BYTES: u64 = 16 * 1024 * 1024;

/// What to do to the file. Paths are relative to the account's root, `/`-separated.
#[derive(Clone, Copy)]
pub enum Edit<'a> {
    Add {
        server_path: &'a str,
        /// The natural order folders are compared with (the same one local favorites use).
        compare: &'a dyn Fn(&str, &str) -> std::cmp::Ordering,
        /// Written as `#EXTINF` only when the file already uses them (or is new).
        title: Option<&'a str>,
        duration_ms: Option<u64>,
    },
    Remove {
        server_path: &'a str,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditError {
    /// The file on the server was changed after it was read; nothing was written.
    Changed,
    Failed(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Changed => write!(
                f,
                "播放清單在伺服器上已被改動，尚未寫入。請重新整理後再試一次。"
            ),
            Self::Failed(message) => write!(f, "{message}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

fn decode(bytes: &[u8], allow_utf16: bool) -> Result<(Encoding, String), String> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        if !allow_utf16 {
            return Err("這份播放清單的編碼不是 UTF-8。".to_owned());
        }
        let little = bytes[0] == 0xFF;
        let body = &bytes[2..];
        if !body.len().is_multiple_of(2) {
            return Err("播放清單的 UTF-16 內容無效。".to_owned());
        }
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|pair| {
                let pair = [pair[0], pair[1]];
                if little {
                    u16::from_le_bytes(pair)
                } else {
                    u16::from_be_bytes(pair)
                }
            })
            .collect();
        let text =
            String::from_utf16(&units).map_err(|_| "播放清單的 UTF-16 內容無效。".to_owned())?;
        return Ok((
            if little {
                Encoding::Utf16Le
            } else {
                Encoding::Utf16Be
            },
            text,
        ));
    }
    let (encoding, body) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (Encoding::Utf8Bom, rest),
        None => (Encoding::Utf8, bytes),
    };
    let text = std::str::from_utf8(body)
        .map_err(|_| "這份播放清單不是 UTF-8 編碼，無法安全地改寫。".to_owned())?;
    Ok((encoding, text.to_owned()))
}

fn encode(encoding: Encoding, text: &str) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Encoding::Utf16Le => [
            &[0xFF, 0xFE][..],
            &text
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>(),
        ]
        .concat(),
        Encoding::Utf16Be => [
            &[0xFE, 0xFF][..],
            &text
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<u8>>(),
        ]
        .concat(),
    }
}

/// One line: byte range of the whole line (with its terminator) and where the content ends.
#[derive(Clone, Copy)]
struct Line {
    start: usize,
    content_end: usize,
    end: usize,
}

fn lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut start = 0;
    for piece in text.split_inclusive('\n') {
        let end = start + piece.len();
        let mut content_end = end;
        if piece.ends_with('\n') {
            content_end -= 1;
            if text[start..content_end].ends_with('\r') {
                content_end -= 1;
            }
        }
        out.push(Line {
            start,
            content_end,
            end,
        });
        start = end;
    }
    out
}

/// A line the reader takes as an entry: not empty, not a `#` line.
fn is_entry(content: &str) -> bool {
    !content.is_empty() && !content.starts_with('#')
}

fn same_path(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn duration_text(duration_ms: Option<u64>) -> String {
    match duration_ms {
        None => "-1".to_owned(),
        Some(ms) if ms.is_multiple_of(1000) => (ms / 1000).to_string(),
        Some(ms) => format!(
            "{}.{}",
            ms / 1000,
            format!("{:03}", ms % 1000).trim_end_matches('0')
        ),
    }
}

/// The entry lines of `text` (their raw content), for learning the style.
pub fn entry_texts(text: &str) -> Vec<&str> {
    lines(text)
        .into_iter()
        .map(|line| &text[line.start..line.content_end])
        .filter(|content| is_entry(content))
        .collect()
}

/// Decode a playlist file's bytes to text the way the editor does (for reading with the same view of the lines).
pub fn decode_text(bytes: &[u8], allow_utf16: bool) -> Result<String, String> {
    decode(bytes, allow_utf16).map(|(_, text)| text)
}

fn edit_text(
    text: &str,
    folder: &str,
    exists: &dyn Fn(&str) -> bool,
    edit: &Edit,
) -> Result<Option<String>, String> {
    let all = lines(text);
    let raws: Vec<&str> = all
        .iter()
        .map(|line| &text[line.start..line.content_end])
        .filter(|content| is_entry(content))
        .collect();
    let style = PlaylistStyle::detect(&raws, folder, exists);
    let names = |server_path: &str| {
        all.iter()
            .enumerate()
            .filter(|(_, line)| is_entry(&text[line.start..line.content_end]))
            .filter(|(_, line)| {
                style
                    .to_server_path(&text[line.start..line.content_end], folder)
                    .is_some_and(|path| same_path(&path, server_path))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>()
    };
    match *edit {
        Edit::Add {
            server_path,
            compare,
            title,
            duration_ms,
        } => {
            if !names(server_path).is_empty() {
                return Ok(None);
            }
            if style.absolute_unmapped() {
                return Err(
                    "無法對應這份清單的路徑格式（清單裡的歌曲在伺服器上找不到），所以不寫入。"
                        .to_owned(),
                );
            }
            let mut entry = style.to_entry_text(server_path, folder);
            if entry.starts_with('#') {
                entry.insert_str(0, "./");
            }
            if entry.contains(['\n', '\r', '\0']) {
                return Err("歌曲路徑含有無法寫入播放清單的字元。".to_owned());
            }
            let eol = if all
                .iter()
                .any(|line| text[line.start..line.end].ends_with("\r\n"))
            {
                "\r\n"
            } else {
                "\n"
            };
            let uses_extinf = all
                .iter()
                .any(|line| text[line.start..line.content_end].starts_with("#EXTINF:"));
            let blank_file = text.trim().is_empty();
            let mut addition = String::new();
            if uses_extinf || blank_file {
                if let Some(title) = title.filter(|title| !title.contains(['\n', '\r'])) {
                    addition.push_str(&format!(
                        "#EXTINF:{},{title}{eol}",
                        duration_text(duration_ms)
                    ));
                } else if uses_extinf {
                    addition.push_str(&format!(
                        "#EXTINF:{},{}{eol}",
                        duration_text(duration_ms),
                        stem(server_path)
                    ));
                }
            }
            addition.push_str(&entry);
            addition.push_str(eol);
            if blank_file {
                return Ok(Some(format!("#EXTM3U{eol}{addition}")));
            }
            // Where it goes is the favorites rule local playlists use (`favorite_insertion_index`), fed with the
            // server paths of the entries; an entry that names no place on the server is not an anchor.
            let entry_lines: Vec<usize> = all
                .iter()
                .enumerate()
                .filter(|(_, line)| is_entry(&text[line.start..line.content_end]))
                .map(|(index, _)| index)
                .collect();
            let keys: Vec<Option<String>> = entry_lines
                .iter()
                .map(|index| {
                    let line = all[*index];
                    style
                        .to_server_path(&text[line.start..line.content_end], folder)
                        .map(|path| format!("/{path}"))
                })
                .collect();
            let (offset, eol_first) = if keys.iter().all(Option::is_none) {
                // Nothing to anchor to: after the last non-blank line.
                let last = all
                    .iter()
                    .rev()
                    .find(|line| line.content_end > line.start)
                    .copied()
                    .expect("a file with text has a non-empty line");
                (last.end, last.end == last.content_end)
            } else {
                let index = player_core::playlist::favorite_insertion_index(
                    &keys,
                    &format!("/{}", server_path.trim_matches('/')),
                    compare,
                );
                if index == 0 {
                    // Before the first entry (and before the `#EXTINF` line that belongs to it).
                    let first = entry_lines[0];
                    let owns_extinf = first > 0
                        && text[all[first - 1].start..all[first - 1].content_end].starts_with("#EXTINF:");
                    (all[if owns_extinf { first - 1 } else { first }].start, false)
                } else {
                    // Directly after the path line of the entry it follows.
                    let anchor = all[entry_lines[index - 1]];
                    (anchor.end, anchor.end == anchor.content_end)
                }
            };
            let mut result = String::with_capacity(text.len() + addition.len() + 2);
            result.push_str(&text[..offset]);
            if eol_first {
                result.push_str(eol);
            }
            result.push_str(&addition);
            result.push_str(&text[offset..]);
            Ok(Some(result))
        }
        Edit::Remove { server_path } => {
            let hits = names(server_path);
            if hits.is_empty() {
                return Ok(None);
            }
            let mut drop = vec![false; all.len()];
            for index in hits {
                drop[index] = true;
                if index > 0
                    && text[all[index - 1].start..all[index - 1].content_end]
                        .starts_with("#EXTINF:")
                {
                    drop[index - 1] = true;
                }
            }
            let mut result = String::with_capacity(text.len());
            for (line, dropped) in all.iter().zip(drop) {
                if !dropped {
                    result.push_str(&text[line.start..line.end]);
                }
            }
            Ok(Some(result))
        }
    }
}

/// Apply `edit` to the bytes of a playlist file that sits in `folder`. `Ok(None)` means nothing needs to change.
pub fn edit_playlist_bytes(
    bytes: &[u8],
    allow_utf16: bool,
    folder: &str,
    exists: &dyn Fn(&str) -> bool,
    edit: &Edit,
) -> Result<Option<Vec<u8>>, String> {
    let (encoding, text) = decode(bytes, allow_utf16)?;
    Ok(edit_text(&text, folder, exists, edit)?.map(|edited| encode(encoding, &edited)))
}

/// Read a whole (small) remote file.
pub fn read_all(fs: &dyn RemoteFs, path: &str) -> Result<Vec<u8>, RemoteError> {
    let mut file = fs.open(path)?;
    if file.len() > MAX_PLAYLIST_BYTES {
        return Err(RemoteError::Protocol("播放清單檔案過大。".to_owned()));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(file.len()).unwrap_or(0));
    let mut offset = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    while offset < file.len() {
        let read = file.read_at(offset, &mut buffer)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        offset += read as u64;
    }
    Ok(bytes)
}

/// The name the previous version of a playlist file is kept under: the file's name plus `.bak` (`x.m3u8.bak`).
pub fn backup_path(path: &str) -> String {
    format!("{path}.bak")
}

/// The folder of a file path (`""` at the root).
pub fn folder_of(path: &str) -> &str {
    path.trim_matches('/')
        .rsplit_once('/')
        .map_or("", |(folder, _)| folder)
}

/// Add or remove a song in the playlist file `file_rel` on `fs`. Returns whether the file was rewritten. The file is
/// read now; it is replaced only if it is still the file that was read ([`EditError::Changed`] otherwise).
pub fn edit_playlist_file(
    fs: &dyn RemoteFs,
    file_rel: &str,
    edit: &Edit,
) -> Result<bool, EditError> {
    let map = |error: RemoteError| match error {
        RemoteError::Changed => EditError::Changed,
        other => EditError::Failed(other.to_string()),
    };
    let entry = fs.stat(file_rel).map_err(map)?;
    if entry.is_dir {
        return Err(EditError::Failed("這不是播放清單檔案。".to_owned()));
    }
    let stamp = FileStamp::of(&entry);
    let bytes = read_all(fs, file_rel).map_err(map)?;
    if bytes.len() as u64 != entry.size {
        return Err(EditError::Changed);
    }
    let allow_utf16 = file_rel.to_ascii_lowercase().ends_with(".m3u");
    let exists = |path: &str| fs.stat(path).is_ok_and(|entry| !entry.is_dir);
    let edited = edit_playlist_bytes(&bytes, allow_utf16, folder_of(file_rel), &exists, edit)
        .map_err(EditError::Failed)?;
    let Some(edited) = edited else {
        return Ok(false);
    };
    let len = edited.len() as u64;
    fs.replace_file_keeping_backup(file_rel, &mut Cursor::new(edited), len, Some(stamp), &backup_path(file_rel))
        .map_err(map)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::DirFs;
    const B: &str = "\\";
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tree(files: &[&str]) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "moe-playlist-edit-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        }
        root
    }

    fn natural(left: &str, right: &str) -> std::cmp::Ordering {
        player_db::natural_title_cmp(left, right)
    }

    fn add(path: &str) -> Edit<'_> {
        Edit::Add {
            server_path: path,
            compare: &natural,
            title: Some("Artist - Song"),
            duration_ms: Some(215_500),
        }
    }

    fn run(root: &PathBuf, file: &str, bytes: &[u8], edit: Edit) -> Result<Vec<u8>, EditError> {
        std::fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
        std::fs::write(root.join(file), bytes).unwrap();
        edit_playlist_file(&DirFs::new(root), file, &edit)?;
        Ok(std::fs::read(root.join(file)).unwrap())
    }

    #[test]
    fn adding_to_an_empty_file_writes_a_header_and_relative_slashes() {
        let root = tree(&["Music/Loss/new.flac"]);
        let out = run(
            &root,
            "Music/Playlists/p.m3u8",
            b"",
            add("Music/Loss/new.flac"),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "#EXTM3U\n#EXTINF:215.5,Artist - Song\n../Loss/new.flac\n"
        );
    }

    #[test]
    fn drive_letter_files_keep_their_style_and_every_other_byte() {
        let root = tree(&[
            "Music/Loss/a.flac",
            "Music/Loss/b.flac",
            "Music/Loss/日本/new song.flac",
        ]);
        let original = "\u{feff}#EXTM3U\r\n#PLAYLIST:Mine\r\n#EXTINF:10,A\r\nD:\\Music\\Loss\\a.flac\r\n; odd comment kept\r\nD:\\Music\\Loss\\b.flac\r\n\r\n";
        let out = run(
            &root,
            "Music/Playlists/p.m3u8",
            original.as_bytes(),
            add("Music/Loss/日本/new song.flac"),
        )
        .unwrap();
        let expected = "\u{feff}#EXTM3U\r\n#PLAYLIST:Mine\r\n#EXTINF:10,A\r\nD:\\Music\\Loss\\a.flac\r\n; odd comment kept\r\nD:\\Music\\Loss\\b.flac\r\n#EXTINF:215.5,Artist - Song\r\nD:\\Music\\Loss\\日本\\new song.flac\r\n\r\n";
        assert_eq!(String::from_utf8(out).unwrap(), expected);
    }

    #[test]
    fn extinf_is_added_only_when_the_file_uses_it() {
        let root = tree(&["Music/a.flac", "Music/b.flac"]);
        let out = run(&root, "Music/p.m3u8", b"a.flac", add("Music/b.flac")).unwrap();
        // The last line had no terminator: it gets one, nothing else of it changes.
        assert_eq!(String::from_utf8(out).unwrap(), "a.flac\nb.flac\n");
    }

    #[test]
    fn relative_backslash_files_stay_backslash() {
        let root = tree(&["Music/Loss/a.flac", "Music/Loss/b.flac"]);
        let out = run(
            &root,
            "Music/Playlists/p.m3u8",
            b"..\\Loss\\a.flac\n",
            add("Music/Loss/b.flac"),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "..\\Loss\\a.flac\n..\\Loss\\b.flac\n"
        );
    }

    #[test]
    fn duplicates_are_not_added_and_the_file_is_not_touched() {
        let root = tree(&["Music/Loss/a.flac"]);
        let file = "Music/Playlists/p.m3u8";
        std::fs::create_dir_all(root.join("Music/Playlists")).unwrap();
        std::fs::write(root.join(file), b"../Loss/a.flac\n").unwrap();
        let before = std::fs::metadata(root.join(file))
            .unwrap()
            .modified()
            .unwrap();
        assert!(!edit_playlist_file(&DirFs::new(&root), file, &add("Music/Loss/a.flac")).unwrap());
        assert_eq!(
            std::fs::metadata(root.join(file))
                .unwrap()
                .modified()
                .unwrap(),
            before
        );
    }

    #[test]
    fn remove_drops_the_entry_and_its_extinf_only() {
        let root = tree(&["Music/Loss/a.flac", "Music/Loss/b.flac"]);
        let original =
            "#EXTM3U\n#EXTINF:1,A\nD:\\Music\\Loss\\a.flac\n#EXTINF:2,B\nD:\\Music\\Loss\\b.flac\n";
        let out = run(
            &root,
            "Music/Playlists/p.m3u8",
            original.as_bytes(),
            Edit::Remove {
                server_path: "Music/Loss/a.flac",
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "#EXTM3U\n#EXTINF:2,B\nD:\\Music\\Loss\\b.flac\n"
        );
        // A song that is not listed changes nothing.
        let same = run(
            &root,
            "Music/Playlists/q.m3u8",
            b"../Loss/b.flac\n",
            Edit::Remove {
                server_path: "Music/Loss/a.flac",
            },
        )
        .unwrap();
        assert_eq!(same, b"../Loss/b.flac\n");
    }

    #[test]
    fn utf16_m3u_keeps_its_encoding() {
        let root = tree(&["Music/a.flac", "Music/b.flac"]);
        let mut original = vec![0xFF, 0xFE];
        original.extend("a.flac\r\n".encode_utf16().flat_map(u16::to_le_bytes));
        let out = run(&root, "Music/p.m3u", &original, add("Music/b.flac")).unwrap();
        let mut expected = vec![0xFF, 0xFE];
        expected.extend(
            "a.flac\r\nb.flac\r\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn a_file_that_is_not_utf8_is_refused_untouched() {
        let root = tree(&["Music/a.flac"]);
        let bytes = [b'a', 0xE9, b'\n'];
        let error = run(&root, "Music/p.m3u8", &bytes, add("Music/a.flac")).unwrap_err();
        assert!(matches!(error, EditError::Failed(_)));
        assert_eq!(std::fs::read(root.join("Music/p.m3u8")).unwrap(), bytes);
    }

    #[test]
    fn unmappable_absolute_entries_are_refused() {
        let root = tree(&["Music/a.flac"]);
        let error = run(
            &root,
            "Music/p.m3u8",
            b"E:\\nowhere\\x.flac\n",
            add("Music/a.flac"),
        )
        .unwrap_err();
        assert!(matches!(error, EditError::Failed(_)));
    }

    /// A file whose server copy changes between the read and the write is refused and left as the other writer made it.
    #[test]
    fn a_changed_file_is_refused_and_left_alone() {
        struct Racing {
            inner: DirFs,
            root: PathBuf,
        }
        impl RemoteFs for Racing {
            fn list(&self, dir: &str) -> Result<Vec<crate::fs::RemoteEntry>, RemoteError> {
                self.inner.list(dir)
            }
            fn stat(&self, path: &str) -> Result<crate::fs::RemoteEntry, RemoteError> {
                self.inner.stat(path)
            }
            fn open(&self, path: &str) -> Result<Box<dyn crate::fs::RemoteFile>, RemoteError> {
                let file = self.inner.open(path);
                // Someone else writes the playlist right after we read it.
                std::fs::write(self.root.join(path), b"../Loss/someone-else.flac\nmore\n").unwrap();
                file
            }
            fn replace_file(
                &self,
                path: &str,
                source: &mut dyn std::io::Read,
                len: u64,
                expected: Option<FileStamp>,
            ) -> Result<crate::fs::RemoteEntry, RemoteError> {
                self.inner.replace_file(path, source, len, expected)
            }
        }
        let root = tree(&["Music/Loss/a.flac"]);
        let file = "Music/Playlists/p.m3u8";
        std::fs::create_dir_all(root.join("Music/Playlists")).unwrap();
        std::fs::write(root.join(file), b"../Loss/a.flac\n").unwrap();
        let fs = Racing {
            inner: DirFs::new(&root),
            root: root.clone(),
        };
        let error = edit_playlist_file(&fs, file, &add("Music/Loss/new.flac")).unwrap_err();
        assert_eq!(error, EditError::Changed);
        assert_eq!(
            std::fs::read(root.join(file)).unwrap(),
            b"../Loss/someone-else.flac\nmore\n"
        );
        // No leftover temporary file.
        assert!(!root.join("Music/Playlists/.moetmp-p.m3u8").exists());
    }

    #[test]
    fn mixed_files_with_blank_trailing_lines_insert_before_them() {
        let root = tree(&["a.flac", "b.flac"]);
        let out = run(&root, "p.m3u8", b"a.flac\n\n\n", add("b.flac")).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "a.flac\nb.flac\n\n\n");
    }

    // ---- where a new song goes: the same rule as local favorites ----

    #[test]
    fn a_song_goes_after_the_last_entry_of_its_folder_and_other_lines_stay() {
        let root = tree(&["Music/A/new.flac"]);
        let original = "#EXTM3U\n#EXTINF:1,x\n../A/one.flac\n; note\n#EXTINF:2,y\n../B/one.flac\n\n../A/two.flac\n; tail\n";
        let out = run(&root, "Music/Playlists/p.m3u8", original.as_bytes(), add("Music/A/new.flac")).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "#EXTM3U\n#EXTINF:1,x\n../A/one.flac\n; note\n#EXTINF:2,y\n../B/one.flac\n\n../A/two.flac\n#EXTINF:215.5,Artist - Song\n../A/new.flac\n; tail\n"
        );
    }

    #[test]
    fn a_new_folder_goes_next_to_the_nearest_parent_folder_group() {
        let root = tree(&[]);
        let original = "../A/x/one.flac\n../B/one.flac\n../A/two.flac\n";
        // `A/y/` has no entry yet: the nearest group is `A/`, whose last entry is `two.flac`.
        let out = run(&root, "Music/Playlists/p.m3u8", original.as_bytes(), add("Music/A/y/new.flac")).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "../A/x/one.flac\n../B/one.flac\n../A/two.flac\n../A/y/new.flac\n");
    }

    #[test]
    fn a_song_in_no_known_group_goes_to_the_end() {
        let root = tree(&[]);
        let out = run(&root, "Music/Playlists/p.m3u8", b"../A/one.flac\n../B/one.flac\n", add("Other/new.flac")).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "../A/one.flac\n../B/one.flac\n../../Other/new.flac\n");
    }

    #[test]
    fn unmappable_entries_are_not_anchors() {
        let root = tree(&[]);
        // The web address and the path that leaves the server are skipped; `A/` is the only group.
        let original = "../A/one.flac\nhttp://example.com/x.mp3\n../../../../up.flac\n";
        let out = run(&root, "Music/Playlists/p.m3u8", original.as_bytes(), add("Music/A/new.flac")).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "../A/one.flac\n../A/new.flac\nhttp://example.com/x.mp3\n../../../../up.flac\n"
        );
    }

    #[test]
    fn index_zero_goes_before_the_first_entry_and_its_extinf() {
        // `Music/A/x` has no entry; the nearest group is the root of the account, which is found at `/a.flac`.
        let keys = [Some("/Z/a.flac".to_owned()), Some("/Z/b.flac".to_owned())];
        assert_eq!(player_core::playlist::favorite_insertion_index(&keys, "/Z/c.flac", &natural), 2);
        // No group at all: the end.
        assert_eq!(player_core::playlist::favorite_insertion_index(&keys, "/Y/c.flac", &natural), 2);
        // Index 0 comes only from an empty list of anchors in front: a group whose entry is the first one.
        let keys = [None, Some("/Z/a.flac".to_owned())];
        assert_eq!(player_core::playlist::favorite_insertion_index(&keys, "/Z/c.flac", &natural), 2);
    }

    #[test]
    fn a_crlf_bom_file_with_extinf_is_extended_byte_for_byte() {
        let root = tree(&[]);
        let original = "\u{feff}#EXTM3U" .to_owned() + "\r\n#EXTINF:1,x\r\nD:" + B + "Music" + B + "A" + B + "one.flac\r\n; c\r\nD:" + B + "Music" + B + "B" + B + "one.flac\r\n";
        let files = ["Music/A/one.flac", "Music/B/one.flac"];
        for f in files {
            std::fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
            std::fs::write(root.join(f), b"x").unwrap();
        }
        let out = run(&root, "Music/Playlists/p.m3u8", original.as_bytes(), add("Music/A/new.flac")).unwrap();
        let expected = "\u{feff}#EXTM3U".to_owned() + "\r\n#EXTINF:1,x\r\nD:" + B + "Music" + B + "A" + B + "one.flac\r\n#EXTINF:215.5,Artist - Song\r\nD:" + B + "Music" + B + "A" + B + "new.flac\r\n; c\r\nD:" + B + "Music" + B + "B" + B + "one.flac\r\n";
        assert_eq!(String::from_utf8(out).unwrap(), expected);
    }

    #[test]
    fn a_song_whose_folder_group_starts_the_list_still_follows_the_group() {
        let root = tree(&[]);
        let out = run(&root, "Music/Playlists/p.m3u8", b"../A/one.flac\n../B/one.flac\n", add("Music/A/new.flac")).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "../A/one.flac\n../A/new.flac\n../B/one.flac\n");
    }

    #[test]
    fn local_and_remote_decide_the_same_index_for_the_same_paths() {
        let paths = ["D:/Music/A/one.flac", "D:/Music/B/one.flac", "D:/Music/A/two.flac", "D:/Other/x.flac"];
        let keys: Vec<Option<String>> = paths.iter().map(|p| Some((*p).to_owned())).collect();
        // The remote side feeds the same paths with a leading slash instead of the drive.
        let remote: Vec<Option<String>> = paths.iter().map(|p| Some(format!("/{}", &p[3..]))).collect();
        for new in ["D:/Music/A/n.flac", "D:/Music/C/n.flac", "D:/Music/A/sub/n.flac", "D:/Other/n.flac", "D:/Else/n.flac"] {
            let remote_new = format!("/{}", &new[3..]);
            assert_eq!(
                player_core::playlist::favorite_insertion_index(&keys, new, &natural),
                player_core::playlist::favorite_insertion_index(&remote, &remote_new, &natural),
                "{new}"
            );
        }
    }

    // ---- the single `.bak` backup ----

    fn bytes_of(root: &std::path::Path, file: &str) -> Vec<u8> {
        std::fs::read(root.join(file)).unwrap()
    }

    fn strays(root: &std::path::Path, folder: &str) -> Vec<String> {
        std::fs::read_dir(root.join(folder))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".moetmp-"))
            .collect()
    }

    #[test]
    fn the_first_edit_keeps_the_previous_version_as_bak_and_a_second_edit_replaces_it() {
        let root = tree(&["Music/A/one.flac", "Music/A/two.flac", "Music/A/three.flac"]);
        let file = "Music/Playlists/p.m3u8";
        let v1 = b"../A/one.flac\n";
        let v2 = run(&root, file, v1, add("Music/A/two.flac")).unwrap();
        assert_eq!(v2, b"../A/one.flac\n../A/two.flac\n");
        assert_eq!(bytes_of(&root, "Music/Playlists/p.m3u8.bak"), v1);
        // A second edit: the backup is the version right before it, and there is only one.
        edit_playlist_file(&DirFs::new(&root), file, &add("Music/A/three.flac")).unwrap();
        assert_eq!(bytes_of(&root, "Music/Playlists/p.m3u8.bak"), v2);
        assert_eq!(bytes_of(&root, file), b"../A/one.flac\n../A/two.flac\n../A/three.flac\n");
        assert!(strays(&root, "Music/Playlists").is_empty());
        assert_eq!(std::fs::read_dir(root.join("Music/Playlists")).unwrap().count(), 2);
    }

    #[test]
    fn a_stale_bak_is_replaced_and_m3u_keeps_its_own_name() {
        let root = tree(&["Music/a.flac", "Music/b.flac"]);
        std::fs::write(root.join("Music/old.m3u.bak"), b"stale").unwrap();
        let out = run(&root, "Music/old.m3u", b"a.flac\n", add("Music/b.flac")).unwrap();
        assert_eq!(out, b"a.flac\nb.flac\n");
        assert_eq!(bytes_of(&root, "Music/old.m3u.bak"), b"a.flac\n");
    }

    #[test]
    fn a_refused_edit_or_a_no_op_creates_no_backup() {
        let root = tree(&["Music/a.flac"]);
        // Duplicate add: nothing is written.
        run(&root, "Music/p.m3u8", b"a.flac\n", add("Music/a.flac")).unwrap();
        assert!(!root.join("Music/p.m3u8.bak").exists());
        // A non-UTF-8 file is refused.
        run(&root, "Music/q.m3u8", &[b'a', 0xE9, b'\n'], add("Music/a.flac")).unwrap_err();
        assert!(!root.join("Music/q.m3u8.bak").exists());
    }

    #[test]
    fn a_changed_file_gets_no_backup_and_a_new_file_has_nothing_to_back_up() {
        let root = tree(&["Music/a.flac"]);
        let fs = DirFs::new(&root);
        std::fs::write(root.join("Music/p.m3u8"), b"a.flac\n").unwrap();
        let stamp = FileStamp::of(&fs.stat("Music/p.m3u8").unwrap());
        std::fs::write(root.join("Music/p.m3u8"), b"someone else wrote this\n").unwrap();
        let refused = fs.replace_file_keeping_backup("Music/p.m3u8", &mut &b"new"[..], 3, Some(stamp), "Music/p.m3u8.bak");
        assert_eq!(refused, Err(RemoteError::Changed));
        assert!(!root.join("Music/p.m3u8.bak").exists());
        assert!(strays(&root, "Music").is_empty());
        // A file that does not exist yet.
        fs.replace_file_keeping_backup("Music/new.m3u8", &mut &b"fresh"[..], 5, None, "Music/new.m3u8.bak").unwrap();
        assert_eq!(bytes_of(&root, "Music/new.m3u8"), b"fresh");
        assert!(!root.join("Music/new.m3u8.bak").exists());
    }

    /// The last rename fails: the original is put back untouched, the old backup is gone (it was replaced by the
    /// original and the original is back), and no temporary file stays.
    #[test]
    fn a_failing_final_rename_rolls_back_to_the_untouched_original() {
        struct Failing(DirFs);
        impl RemoteFs for Failing {
            fn list(&self, dir: &str) -> Result<Vec<crate::fs::RemoteEntry>, RemoteError> {
                self.0.list(dir)
            }
            fn stat(&self, path: &str) -> Result<crate::fs::RemoteEntry, RemoteError> {
                self.0.stat(path)
            }
            fn open(&self, path: &str) -> Result<Box<dyn crate::fs::RemoteFile>, RemoteError> {
                self.0.open(path)
            }
            fn replace_file(
                &self,
                path: &str,
                source: &mut dyn std::io::Read,
                len: u64,
                expected: Option<FileStamp>,
            ) -> Result<crate::fs::RemoteEntry, RemoteError> {
                self.0.replace_file(path, source, len, expected)
            }
            fn remove_file(&self, path: &str) -> Result<(), RemoteError> {
                self.0.remove_file(path)
            }
            fn rename_file(&self, from: &str, to: &str) -> Result<(), RemoteError> {
                if from.contains(".moetmp-") {
                    return Err(RemoteError::Network("link dropped".to_owned()));
                }
                self.0.rename_file(from, to)
            }
        }
        let root = tree(&["Music/a.flac", "Music/b.flac"]);
        let file = "Music/Playlists/p.m3u8";
        std::fs::create_dir_all(root.join("Music/Playlists")).unwrap();
        std::fs::write(root.join(file), b"../a.flac\n").unwrap();
        let before = std::fs::metadata(root.join(file)).unwrap().modified().unwrap();
        let error = edit_playlist_file(&Failing(DirFs::new(&root)), file, &add("Music/b.flac")).unwrap_err();
        assert!(matches!(error, EditError::Failed(_)));
        assert_eq!(bytes_of(&root, file), b"../a.flac\n");
        assert_eq!(std::fs::metadata(root.join(file)).unwrap().modified().unwrap(), before);
        assert!(strays(&root, "Music/Playlists").is_empty());
        assert!(!root.join("Music/Playlists/p.m3u8.bak").exists());
    }
}
