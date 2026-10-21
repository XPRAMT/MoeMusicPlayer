//! How the entries of a playlist file name their songs, and the conversion in both directions (ADR 0026).
//!
//! A playlist made on another computer lists songs in its own style: `D:\Music\Loss\a.flac`, `..\Loss\a.flac`,
//! `Loss/a.flac`, `file:///D:/Music/a.flac`. The server shows the same tree from some other root, so reading has to map
//! such an entry to a path under the account's root ([`PlaylistStyle::to_server_path`]), and writing a new entry has to
//! produce text in the style the file already uses ([`PlaylistStyle::to_entry_text`]). Both directions live here, driven
//! by one [`PlaylistStyle`] learned from the entries already in the file ([`PlaylistStyle::detect`]), so they cannot
//! drift apart. All functions are pure (the server is only asked through the `exists` closure when learning).
//!
//! The style model:
//! - **Relative** entries are resolved against the playlist's folder (`ups` leading `..` pieces climb out of it). When
//!   writing, the shortest relative path is produced, except that a file whose relative entries all climb the same
//!   number of folders keeps climbing that many (`fixed_ups`), a file whose entries start with `./` keeps that, and
//!   the separator (`/` or `\`) is the one the entries use.
//! - **Absolute** entries (`D:\…`, `/srv/…`, `\\host\share\…`, `file:///D:/…`) carry a `lead` (everything before the
//!   first piece, with the drive letter's case) and a separator. The way back to the server is a [`MapRule`]: how many
//!   leading pieces to drop (`dropped` remembers them, to put them back), and which folder above the playlist to put in
//!   front (`prefix`), found once by asking the server which candidate really is a file.
//! - Entries may be percent-encoded (`file://` URIs always are); `raw_subdelims` keeps `( ) ' …` unescaped when the file
//!   does so.

/// One entry line, taken apart. `Other` is anything that is not a path (a web address, a custom scheme).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Relative {
        /// Leading `..` pieces that climb out of the playlist's folder.
        ups: usize,
        pieces: Vec<String>,
        /// The entry started with `./`.
        dot: bool,
        /// The separator it uses, `None` when it has only one piece.
        sep: Option<char>,
    },
    Absolute {
        /// Everything before the first piece: `D:\`, `/`, `\\`, `file:///D:/`.
        lead: String,
        pieces: Vec<String>,
        sep: char,
    },
    Other,
}

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

const SUBDELIMS: &[u8] = b"!$&'()*+,;=:@";

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Percent-encode one path piece.
pub fn encode_piece(piece: &str, keep_subdelims: bool) -> String {
    let mut out = String::with_capacity(piece.len());
    for &b in piece.as_bytes() {
        if is_unreserved(b) || (keep_subdelims && SUBDELIMS.contains(&b)) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode `%XX` sequences; `None` when the result is not UTF-8. A `%` that is not followed by two hex digits stays.
pub fn decode_piece(piece: &str) -> Option<String> {
    let bytes = piece.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn has_percent_escape(text: &str) -> bool {
    let b = text.as_bytes();
    (0..b.len()).any(|i| {
        b[i] == b'%'
            && i + 2 < b.len()
            && b[i + 1].is_ascii_hexdigit()
            && b[i + 2].is_ascii_hexdigit()
    })
}

fn is_file_uri(raw: &str) -> bool {
    raw.get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
}

fn is_other_scheme(raw: &str) -> bool {
    let Some((scheme, _)) = raw.split_once(':') else {
        return false;
    };
    scheme.len() >= 2
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

fn decode_all(pieces: Vec<&str>, percent: bool) -> Option<Vec<String>> {
    pieces
        .into_iter()
        .map(|p| {
            if percent {
                decode_piece(p)
            } else {
                Some(p.to_owned())
            }
        })
        .collect()
}

/// Take one entry line apart. `percent` says the (non-URI) entries are percent-encoded.
pub fn classify(raw: &str, percent: bool) -> Shape {
    if is_file_uri(raw) {
        let rest = &raw[7..];
        let host_end = rest.find('/').unwrap_or(rest.len());
        let host = &rest[..host_end];
        let path = &rest[host_end..];
        if !path.starts_with('/') {
            return Shape::Other;
        }
        let mut lead_len = 7 + host_end + 1;
        let after = &path[1..];
        let bytes = after.as_bytes();
        if bytes.len() >= 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes.len() == 2 || bytes[2] == b'/')
        {
            lead_len += if bytes.len() == 2 { 2 } else { 3 };
        }
        let _ = host;
        let lead = raw[..lead_len.min(raw.len())].to_owned();
        let tail = &raw[lead.len()..];
        let Some(pieces) = decode_all(tail.split('/').filter(|p| !p.is_empty()).collect(), true)
        else {
            return Shape::Other;
        };
        return Shape::Absolute {
            lead,
            pieces,
            sep: '/',
        };
    }
    let bytes = raw.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if !drive && is_other_scheme(raw) {
        return Shape::Other;
    }
    if drive {
        let mut lead_len = 2;
        let mut sep = '\\';
        if let Some(c) = raw[2..].chars().next().filter(|c| is_sep(*c)) {
            lead_len += 1;
            sep = c;
        }
        let Some(pieces) = decode_all(
            raw[lead_len..]
                .split(is_sep)
                .filter(|p| !p.is_empty() && *p != ".")
                .collect(),
            percent,
        ) else {
            return Shape::Other;
        };
        return Shape::Absolute {
            lead: raw[..lead_len].to_owned(),
            pieces,
            sep,
        };
    }
    if raw.starts_with(is_sep) {
        let lead_len = raw.find(|c| !is_sep(c)).unwrap_or(raw.len());
        let sep = raw.chars().next().unwrap_or('/');
        let Some(pieces) = decode_all(
            raw[lead_len..]
                .split(is_sep)
                .filter(|p| !p.is_empty() && *p != ".")
                .collect(),
            percent,
        ) else {
            return Shape::Other;
        };
        return Shape::Absolute {
            lead: raw[..lead_len].to_owned(),
            pieces,
            sep,
        };
    }
    // Relative.
    let sep = raw.chars().find(|c| is_sep(*c));
    let dot = raw.starts_with("./") || raw.starts_with(".\\");
    let Some(parts) = decode_all(raw.split(is_sep).collect(), percent) else {
        return Shape::Other;
    };
    let mut ups = 0;
    let mut pieces: Vec<String> = Vec::new();
    for part in parts {
        match part.as_str() {
            "" | "." => {}
            ".." => {
                if pieces.pop().is_none() {
                    ups += 1;
                }
            }
            _ => pieces.push(part),
        }
    }
    Shape::Relative {
        ups,
        pieces,
        dot,
        sep,
    }
}

/// How an absolute entry reaches the server: drop the first `drop` pieces (they are `dropped`), put `prefix` (a folder
/// of the account, `/`-separated, may be empty) in front.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapRule {
    pub drop: usize,
    pub dropped: Vec<String>,
    pub prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbsoluteStyle {
    pub lead: String,
    pub sep: char,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistStyle {
    pub rel_sep: char,
    pub dot: bool,
    pub fixed_ups: Option<usize>,
    pub percent: bool,
    pub raw_subdelims: bool,
    pub absolute: Option<AbsoluteStyle>,
    pub rule: Option<MapRule>,
    /// Most entries of the file are absolute: new entries are written that way.
    pub prefer_absolute: bool,
}

impl Default for PlaylistStyle {
    /// A file with nothing to learn from: paths relative to the playlist's folder, `/`-separated.
    fn default() -> Self {
        Self {
            rel_sep: '/',
            dot: false,
            fixed_ups: None,
            percent: false,
            raw_subdelims: false,
            absolute: None,
            rule: None,
            prefer_absolute: false,
        }
    }
}

fn split_path(path: &str) -> Vec<&str> {
    path.split('/').filter(|p| !p.is_empty()).collect()
}

/// The folders of `folder` from the root down, as candidates for "the folder a drive maps to".
fn folder_prefixes(folder: &str) -> Vec<String> {
    let mut prefixes = vec![String::new()];
    let mut walked = String::new();
    for piece in split_path(folder) {
        walked = if walked.is_empty() {
            piece.to_owned()
        } else {
            format!("{walked}/{piece}")
        };
        prefixes.push(walked.clone());
    }
    prefixes
}

fn join_rule(pieces: &[String], drop: usize, prefix: &str) -> Option<String> {
    let rest = pieces.get(drop..).filter(|rest| !rest.is_empty())?;
    Some(if prefix.is_empty() {
        rest.join("/")
    } else {
        format!("{prefix}/{}", rest.join("/"))
    })
}

impl PlaylistStyle {
    /// Learn the style from the entry lines of a file in `folder` (the playlist's folder, relative to the account's
    /// root). `exists` says whether a path relative to the account's root is a file on the server; it is only asked while
    /// looking for the mapping rule of absolute entries.
    pub fn detect(raws: &[&str], folder: &str, exists: &dyn Fn(&str) -> bool) -> Self {
        // Percent-encoding: some entry has an escape and no entry has a raw space or non-ASCII letter.
        let percent = raws
            .iter()
            .any(|r| !is_file_uri(r) && has_percent_escape(r))
            && !raws
                .iter()
                .any(|r| !is_file_uri(r) && (r.contains(' ') || !r.is_ascii()));
        let shapes: Vec<Shape> = raws.iter().map(|raw| classify(raw, percent)).collect();
        let mut style = Self {
            percent,
            ..Self::default()
        };

        style.raw_subdelims = raws
            .iter()
            .filter(|r| is_file_uri(r) || percent)
            .any(|r| r.bytes().any(|b| SUBDELIMS.contains(&b) && b != b':'));

        let relatives: Vec<(usize, bool, Option<char>)> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Relative { ups, dot, sep, .. } => Some((*ups, *dot, *sep)),
                _ => None,
            })
            .collect();
        let backslashes = relatives.iter().filter(|r| r.2 == Some('\\')).count();
        let slashes = relatives.iter().filter(|r| r.2 == Some('/')).count();
        style.rel_sep = if backslashes > slashes { '\\' } else { '/' };
        let flat: Vec<_> = relatives.iter().filter(|r| r.0 == 0).collect();
        style.dot = !flat.is_empty() && flat.iter().all(|r| r.1);
        if let Some(first) = relatives.first() {
            if first.0 >= 1 && relatives.iter().all(|r| r.0 == first.0) {
                style.fixed_ups = Some(first.0);
            }
        }

        let absolutes: Vec<(&String, &Vec<String>, char)> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Absolute { lead, pieces, sep } => Some((lead, pieces, *sep)),
                _ => None,
            })
            .collect();
        if let Some((lead, _, sep)) = absolutes.first() {
            style.absolute = Some(AbsoluteStyle {
                lead: (*lead).clone(),
                sep: *sep,
            });
        }
        style.prefer_absolute = absolutes.len() > relatives.len();

        // The mapping rule: the first absolute entry that matches a real file decides.
        let prefixes = folder_prefixes(folder);
        let mut misses = 0_u32;
        for (_, pieces, _) in &absolutes {
            if misses >= 3 {
                break;
            }
            let mut found = None;
            'search: for drop in 0..pieces.len() {
                for prefix in &prefixes {
                    if let Some(candidate) = join_rule(pieces, drop, prefix) {
                        if exists(&candidate) {
                            found = Some(MapRule {
                                drop,
                                dropped: pieces[..drop].to_vec(),
                                prefix: prefix.clone(),
                            });
                            break 'search;
                        }
                    }
                }
            }
            if found.is_none() {
                misses += 1;
            }
            if found.is_some() {
                style.rule = found;
                break;
            }
        }
        style
    }

    /// The path under the account's root an entry names, `None` when it does not name a place on the server (a web
    /// address, a path that climbs out of the root, an absolute path with no known rule).
    pub fn to_server_path(&self, raw: &str, folder: &str) -> Option<String> {
        match classify(raw, self.percent) {
            Shape::Relative { ups, pieces, .. } => {
                let mut base: Vec<&str> = split_path(folder);
                for _ in 0..ups {
                    base.pop()?;
                }
                base.extend(pieces.iter().map(String::as_str));
                (!pieces.is_empty()).then(|| base.join("/"))
            }
            Shape::Absolute { pieces, .. } => {
                let rule = self.rule.as_ref()?;
                join_rule(&pieces, rule.drop, &rule.prefix)
            }
            Shape::Other => None,
        }
    }

    fn encode(&self, piece: &str, uri: bool) -> String {
        if self.percent || uri {
            encode_piece(piece, self.raw_subdelims)
        } else {
            piece.to_owned()
        }
    }

    /// Whether `to_entry_text` can write an absolute entry for `server_path`.
    fn absolute_text(&self, server_path: &str) -> Option<String> {
        let (abs, rule) = (self.absolute.as_ref()?, self.rule.as_ref()?);
        let target = split_path(server_path);
        let prefix = split_path(&rule.prefix);
        if target.len() <= prefix.len() || target[..prefix.len()] != prefix[..] {
            return None;
        }
        let uri = is_file_uri(&abs.lead);
        let sep = abs.sep.to_string();
        let pieces: Vec<String> = rule
            .dropped
            .iter()
            .map(String::as_str)
            .chain(target[prefix.len()..].iter().copied())
            .map(|p| self.encode(p, uri))
            .collect();
        Some(format!("{}{}", abs.lead, pieces.join(&sep)))
    }

    fn relative_text(&self, server_path: &str, folder: &str) -> String {
        let target = split_path(server_path);
        let base = split_path(folder);
        let name_dirs = &target[..target.len().saturating_sub(1)];
        let (ups, rest): (usize, Vec<&str>) = match self.fixed_ups {
            Some(k)
                if base.len() >= k
                    && target.len() > base.len() - k
                    && target[..base.len() - k] == base[..base.len() - k] =>
            {
                (k, target[base.len() - k..].to_vec())
            }
            _ => {
                let common = base
                    .iter()
                    .zip(name_dirs.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                (base.len() - common, target[common..].to_vec())
            }
        };
        let mut parts: Vec<String> = Vec::new();
        if ups == 0 && self.dot {
            parts.push(".".to_owned());
        }
        parts.extend(std::iter::repeat_n("..".to_owned(), ups));
        parts.extend(rest.iter().map(|p| self.encode(p, false)));
        parts.join(&self.rel_sep.to_string())
    }

    /// The text of the entry that names `server_path` (relative to the account's root) in a file of this style, which
    /// sits in `folder`. The inverse of [`to_server_path`](Self::to_server_path).
    pub fn to_entry_text(&self, server_path: &str, folder: &str) -> String {
        if self.prefer_absolute {
            if let Some(text) = self.absolute_text(server_path) {
                return text;
            }
        }
        self.relative_text(server_path, folder)
    }

    /// Whether this file's absolute entries cannot be mapped (a new entry could not be written in their style).
    pub fn absolute_unmapped(&self) -> bool {
        self.prefer_absolute && self.rule.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_probe(_: &str) -> bool {
        false
    }

    /// The server tree the samples map onto.
    fn exists_in<'a>(files: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |path| files.contains(&path)
    }

    fn round_trip(raws: &[&str], folder: &str, files: &[&str]) {
        let style = PlaylistStyle::detect(raws, folder, &exists_in(files));
        for raw in raws {
            let server = style
                .to_server_path(raw, folder)
                .unwrap_or_else(|| panic!("{raw:?} did not map ({style:?})"));
            assert_eq!(
                &style.to_entry_text(&server, folder),
                raw,
                "style {style:?}"
            );
        }
    }

    #[test]
    fn drive_letter_entries_round_trip() {
        let raws = [
            r"D:\Music\Loss\a.flac",
            r"D:\Music\Loss\[x] b (1).flac",
            "D:\\Music\\Loss\\日本語\\歌 ひらがな.flac",
            "D:\\Music\\Loss\\ lead and trail .flac",
        ];
        let files = [
            "Music/Loss/a.flac",
            "Music/Loss/[x] b (1).flac",
            "Music/Loss/日本語/歌 ひらがな.flac",
            "Music/Loss/ lead and trail .flac",
        ];
        round_trip(&raws, "Music/Playlists", &files);
    }

    #[test]
    fn a_drive_that_maps_below_the_share_puts_the_dropped_folders_back() {
        // The share is `Music` itself: `D:\Music\` is dropped.
        let raws = [r"D:\Music\Loss\a.flac", r"D:\Music\Loss\b.flac"];
        let style = PlaylistStyle::detect(
            &raws,
            "Playlists",
            &exists_in(&["Loss/a.flac", "Loss/b.flac"]),
        );
        let rule = style.rule.clone().unwrap();
        assert_eq!((rule.drop, rule.prefix.as_str()), (1, ""));
        assert_eq!(
            style.to_server_path(raws[1], "Playlists").unwrap(),
            "Loss/b.flac"
        );
        assert_eq!(
            style.to_entry_text("Loss/new.flac", "Playlists"),
            r"D:\Music\Loss\new.flac"
        );
    }

    #[test]
    fn drive_letter_case_and_forward_slashes_are_kept() {
        round_trip(
            &["d:/Music/Loss/a.flac", "d:/Music/Loss/b.flac"],
            "Lists",
            &["Music/Loss/a.flac", "Music/Loss/b.flac"],
        );
    }

    #[test]
    fn posix_and_unc_entries_round_trip() {
        round_trip(
            &["/srv/media/Loss/a.flac", "/srv/media/Loss/b.flac"],
            "Lists",
            &["Loss/a.flac", "Loss/b.flac"],
        );
        round_trip(
            &[r"\\nas\music\Loss\a.flac", r"\\nas\music\Loss\b.flac"],
            "Lists",
            &["Loss/a.flac", "Loss/b.flac"],
        );
    }

    #[test]
    fn relative_entries_round_trip_with_both_separators() {
        let files: [&str; 0] = [];
        round_trip(
            &["../Loss/a.flac", "../Loss/日本/b c.flac"],
            "Music/Playlists",
            &files,
        );
        round_trip(
            &[r"..\Loss\a.flac", r"..\Loss\b.flac"],
            "Music/Playlists",
            &files,
        );
        round_trip(&["Loss/a.flac", "a.flac", "Loss/x/b.flac"], "Music", &files);
        round_trip(&["./a.flac", "./sub/b.flac"], "Music/Playlists", &files);
        // Every entry climbs the same two folders (back to the account's root).
        round_trip(
            &["../../Music/Loss/a.flac", "../../Music/Loss/b.flac"],
            "Music/Playlists",
            &files,
        );
    }

    #[test]
    fn percent_encoded_and_file_uri_entries_round_trip() {
        let files = ["Music/Loss/a b.flac", "Music/Loss/日本.flac"];
        round_trip(
            &[
                "file:///D:/Music/Loss/a%20b.flac",
                "file:///D:/Music/Loss/%E6%97%A5%E6%9C%AC.flac",
            ],
            "Music/Lists",
            &files,
        );
        round_trip(
            &["../Loss/a%20b.flac", "../Loss/%E6%97%A5%E6%9C%AC.flac"],
            "Music/Lists",
            &[],
        );
    }

    #[test]
    fn plain_percent_signs_in_names_are_not_decoded() {
        let style = PlaylistStyle::detect(
            &["../100%25 real.flac", "../50% off.flac"],
            "a/b",
            &no_probe,
        );
        assert!(!style.percent);
        assert_eq!(
            style.to_server_path("../50% off.flac", "a/b").unwrap(),
            "a/50% off.flac"
        );
    }

    #[test]
    fn nothing_to_learn_means_relative_with_slashes() {
        let style = PlaylistStyle::detect(&[], "Music/Playlists", &no_probe);
        assert_eq!(
            style.to_entry_text("Music/Loss/a.flac", "Music/Playlists"),
            "../Loss/a.flac"
        );
        assert_eq!(
            style.to_entry_text("Music/Playlists/a.flac", "Music/Playlists"),
            "a.flac"
        );
        assert_eq!(
            style.to_entry_text("Other/a.flac", "Music/Playlists"),
            "../../Other/a.flac"
        );
        assert_eq!(style.to_entry_text("a.flac", ""), "a.flac");
    }

    #[test]
    fn new_entries_outside_the_fixed_anchor_fall_back_to_the_shortest_path() {
        let style = PlaylistStyle::detect(&["../Loss/a.flac"], "Music/Playlists", &no_probe);
        assert_eq!(style.fixed_ups, Some(1));
        assert_eq!(
            style.to_entry_text("Music/Playlists/z.flac", "Music/Playlists"),
            "../Playlists/z.flac"
        );
        assert_eq!(
            style.to_entry_text("Elsewhere/z.flac", "Music/Playlists"),
            "../../Elsewhere/z.flac"
        );
    }

    #[test]
    fn a_path_outside_the_absolute_prefix_is_written_relative() {
        let raws = [r"E:\Songs\a.flac"];
        let style = PlaylistStyle::detect(&raws, "Lists", &exists_in(&["Lists/Songs/a.flac"]));
        assert_eq!(style.rule.as_ref().unwrap().prefix, "Lists");
        assert_eq!(
            style.to_entry_text("Lists/Songs/b.flac", "Lists"),
            r"E:\Songs\b.flac"
        );
        assert_eq!(
            style.to_entry_text("Other/b.flac", "Lists"),
            "../Other/b.flac"
        );
    }

    #[test]
    fn unmapped_absolute_entries_are_reported() {
        let style = PlaylistStyle::detect(&[r"E:\gone\a.flac"], "Lists", &no_probe);
        assert!(style.absolute_unmapped());
        assert!(style.to_server_path(r"E:\gone\a.flac", "Lists").is_none());
    }

    #[test]
    fn web_addresses_and_custom_schemes_are_not_paths() {
        let style = PlaylistStyle::default();
        assert!(style
            .to_server_path("http://example.com/a.mp3", "x")
            .is_none());
        assert!(style.to_server_path("rtsp://cam/stream", "x").is_none());
        assert!(style.to_server_path("../../up.flac", "a").is_none());
    }
}
