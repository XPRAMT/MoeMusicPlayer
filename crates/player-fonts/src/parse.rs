//! One font file's faces: names, writing systems, weight and style (ADR 0042).

use crate::names::LocalizedName;
use crate::scripts::{detect_scripts, Scripts};
use crate::sfnt;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;
use ttf_parser::{cmap, name, name_id, os2, PlatformId};

/// Largest `name` / `OS/2` table read.
const MAX_NAME_TABLE: u32 = 4 << 20;
/// Largest `cmap` table read (a full CJK font's is a few MB).
const MAX_CMAP_TABLE: u32 = 32 << 20;

/// The localized names of one face.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceNames {
    /// Typographic family (name ID 16), else the family (ID 1): faces with the same family are one picker entry.
    pub family: Vec<LocalizedName>,
    /// Typographic subfamily (ID 17), else the subfamily (ID 2).
    pub subfamily: Vec<LocalizedName>,
    /// Full name (ID 4).
    pub full_name: Vec<LocalizedName>,
}

/// One face of a font file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceInfo {
    /// Index of the face in a collection (0 for a plain font).
    pub face_index: u32,
    pub names: FaceNames,
    /// `OS/2` usWeightClass (400 without the table).
    pub weight: u16,
    pub italic: bool,
    pub scripts: Scripts,
}

/// The faces of the font file at `path` (faces without a usable family name are left out).
pub fn read_font_file(path: &Path) -> io::Result<Vec<FaceInfo>> {
    let mut reader = BufReader::new(File::open(path)?);
    let offsets = sfnt::face_offsets(&mut reader)?;
    let mut faces = Vec::new();
    for (index, offset) in offsets.into_iter().enumerate() {
        let Ok(records) = sfnt::table_records(&mut reader, offset) else {
            continue;
        };
        let Some(name) = sfnt::read_table(&mut reader, &records, b"name", MAX_NAME_TABLE)? else {
            continue;
        };
        let cmap = sfnt::read_table(&mut reader, &records, b"cmap", MAX_CMAP_TABLE)?;
        let os2 = sfnt::read_table(&mut reader, &records, b"OS/2", MAX_NAME_TABLE)?;
        if let Some(face) = face_from_tables(index as u32, &name, cmap.as_deref(), os2.as_deref()) {
            faces.push(face);
        }
    }
    Ok(faces)
}

/// A face from its raw `name`, `cmap` and `OS/2` tables; `None` without a family name.
pub fn face_from_tables(
    face_index: u32,
    name_table: &[u8],
    cmap_table: Option<&[u8]>,
    os2_table: Option<&[u8]>,
) -> Option<FaceInfo> {
    let table = name::Table::parse(name_table)?;
    let collect = |id: u16| -> Vec<LocalizedName> {
        let mut out: Vec<LocalizedName> = Vec::new();
        for record in table
            .names
            .into_iter()
            .filter(|record| record.name_id == id)
        {
            let entry = match record.platform_id {
                PlatformId::Windows => record.to_string().map(|name| (record.language_id, name)),
                // Macintosh Roman English: plain ASCII is the same in Mac Roman.
                PlatformId::Macintosh if record.encoding_id == 0 && record.language_id == 0 => {
                    record
                        .name
                        .is_ascii()
                        .then(|| (0x0409, String::from_utf8_lossy(record.name).into_owned()))
                }
                PlatformId::Unicode => record.to_string().map(|name| (0, name)),
                _ => None,
            };
            let Some((lang, value)) = entry else { continue };
            let value = value.trim().trim_matches('\0').to_owned();
            if value.is_empty() || out.iter().any(|existing| existing.lang == lang) {
                continue;
            }
            out.push(LocalizedName { lang, name: value });
        }
        out
    };
    let prefer = |primary: u16, fallback: u16| {
        let names = collect(primary);
        if names.is_empty() {
            collect(fallback)
        } else {
            names
        }
    };
    let names = FaceNames {
        family: prefer(name_id::TYPOGRAPHIC_FAMILY, name_id::FAMILY),
        subfamily: prefer(name_id::TYPOGRAPHIC_SUBFAMILY, name_id::SUBFAMILY),
        full_name: collect(name_id::FULL_NAME),
    };
    if names.family.is_empty() {
        return None;
    }
    let (weight, italic) = os2_table
        .and_then(os2::Table::parse)
        .map(|os2| {
            (
                os2.weight().to_number(),
                !matches!(os2.style(), ttf_parser::Style::Normal),
            )
        })
        .unwrap_or((400, false));
    let scripts = cmap_table
        .and_then(cmap::Table::parse)
        .map(|cmap| {
            let subtables: Vec<_> = cmap
                .subtables
                .into_iter()
                .filter(|subtable| subtable.is_unicode())
                .collect();
            detect_scripts(|c| {
                subtables
                    .iter()
                    .any(|subtable| subtable.glyph_index(u32::from(c)).is_some_and(|g| g.0 != 0))
            })
        })
        .unwrap_or_default();
    Some(FaceInfo {
        face_index,
        names,
        weight,
        italic,
        scripts,
    })
}

/// Test helpers that build `name`, `cmap` and `OS/2` tables.
#[cfg(test)]
pub mod testing {
    /// A format-0 `name` table of Windows Unicode BMP records `(name id, language, text)`.
    pub fn name_table(records: &[(u16, u16, &str)]) -> Vec<u8> {
        let mut strings = Vec::new();
        let mut entries = Vec::new();
        for &(id, lang, text) in records {
            let encoded: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
            entries.push((id, lang, strings.len() as u16, encoded.len() as u16));
            strings.extend_from_slice(&encoded);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_be_bytes());
        out.extend_from_slice(&((6 + 12 * entries.len()) as u16).to_be_bytes());
        for (id, lang, offset, length) in entries {
            for value in [3u16, 1, lang, id, length, offset] {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        out.extend_from_slice(&strings);
        out
    }

    /// A `cmap` with one format-12 (3, 10) subtable mapping each of `chars` to its own glyph.
    pub fn cmap_table(chars: &str) -> Vec<u8> {
        let mut codes: Vec<u32> = chars.chars().map(u32::from).collect();
        codes.sort_unstable();
        codes.dedup();
        let mut sub = Vec::new();
        sub.extend_from_slice(&12u16.to_be_bytes());
        sub.extend_from_slice(&0u16.to_be_bytes());
        sub.extend_from_slice(&((16 + 12 * codes.len()) as u32).to_be_bytes());
        sub.extend_from_slice(&0u32.to_be_bytes());
        sub.extend_from_slice(&(codes.len() as u32).to_be_bytes());
        for (glyph, code) in codes.iter().enumerate() {
            sub.extend_from_slice(&code.to_be_bytes());
            sub.extend_from_slice(&code.to_be_bytes());
            sub.extend_from_slice(&(glyph as u32 + 1).to_be_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&3u16.to_be_bytes());
        out.extend_from_slice(&10u16.to_be_bytes());
        out.extend_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(&sub);
        out
    }

    /// A version-4 `OS/2` table with `weight` and the italic bit.
    pub fn os2_table(weight: u16, italic: bool) -> Vec<u8> {
        let mut out = vec![0u8; 96];
        out[0..2].copy_from_slice(&4u16.to_be_bytes());
        out[4..6].copy_from_slice(&weight.to_be_bytes());
        out[6..8].copy_from_slice(&5u16.to_be_bytes()); // width: normal
        let selection: u16 = if italic { 1 } else { 0x40 };
        out[62..64].copy_from_slice(&selection.to_be_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::sfnt::build_font;

    #[test]
    fn typographic_names_win_and_localized_records_are_kept() {
        let name = name_table(&[
            (1, 0x0409, "Example Semibold"),
            (2, 0x0409, "Regular"),
            (16, 0x0409, "Example"),
            (16, 0x0404, "範例體"),
            (17, 0x0409, "Semibold"),
            (4, 0x0409, "Example Semibold"),
        ]);
        let face = face_from_tables(2, &name, None, None).unwrap();
        assert_eq!(face.face_index, 2);
        let family: Vec<_> = face
            .names
            .family
            .iter()
            .map(|n| (n.lang, n.name.as_str()))
            .collect();
        assert_eq!(family, vec![(0x0409, "Example"), (0x0404, "範例體")]);
        assert_eq!(face.names.subfamily[0].name, "Semibold");
        assert_eq!(face.names.full_name[0].name, "Example Semibold");
        assert_eq!((face.weight, face.italic), (400, false), "no OS/2: regular");
        assert_eq!(face.scripts, Scripts(0), "no cmap: Latin only");

        let legacy = name_table(&[(1, 0x0409, "Old Font"), (2, 0x0409, "Bold")]);
        let face = face_from_tables(0, &legacy, None, None).unwrap();
        assert_eq!(face.names.family[0].name, "Old Font");
        assert_eq!(face.names.subfamily[0].name, "Bold");
        assert!(face_from_tables(0, &name_table(&[(4, 0x0409, "x")]), None, None).is_none());
    }

    #[test]
    fn weight_style_and_scripts_come_from_os2_and_cmap() {
        let name = name_table(&[(1, 0x0409, "Kana Sans")]);
        let cmap = cmap_table("Aaあのをアンヴ");
        let os2 = os2_table(700, true);
        let face = face_from_tables(0, &name, Some(&cmap), Some(&os2)).unwrap();
        assert_eq!((face.weight, face.italic), (700, true));
        assert_eq!(face.scripts, Scripts(Scripts::JAPANESE));
    }

    #[test]
    fn reads_every_face_of_a_collection_file() {
        let face = |family: &str, chars: &str| {
            vec![
                (*b"name", name_table(&[(1, 0x0409, family)])),
                (*b"cmap", cmap_table(chars)),
                (*b"OS/2", os2_table(400, false)),
            ]
        };
        let bytes = build_font(&[
            face("Han TC", "體邊說們這嗎妳戶ㄅㄆㄇㄈ"),
            face("Han SC", "体边说们这吗书东"),
        ]);
        let directory = std::env::temp_dir().join(format!("player-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("collection.ttc");
        std::fs::write(&path, bytes).unwrap();
        let faces = read_font_file(&path).unwrap();
        std::fs::remove_dir_all(&directory).ok();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0].names.family[0].name, "Han TC");
        assert_eq!(faces[0].scripts, Scripts(Scripts::TRADITIONAL));
        assert_eq!(faces[1].face_index, 1);
        assert_eq!(faces[1].scripts, Scripts(Scripts::SIMPLIFIED));
    }
}
