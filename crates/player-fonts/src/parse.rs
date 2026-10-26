//! One font file's faces: names, writing systems, weight and style (ADR 0042).

use crate::names::LocalizedName;
use crate::scripts::{assess_coverage, is_cjk_mark, legacy_encoding_scripts, Coverage, Scripts};
use crate::sfnt;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;
use ttf_parser::{cmap, name, name_id, os2, PlatformId};

/// The version of what `read_font_file` derives from a font file. A cache made by another version is stale and its
/// files are reparsed (2: threshold coverage tests, legacy-encoding character maps, hidden faces; 3: the 200
/// distinctive Traditional / Simplified forms and ㄅ ˙; 4: ㄅ alone).
pub const PARSE_VERSION: u32 = 4;

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
    /// Not offered in the pickers (it cannot show ordinary text in the writing systems it has glyphs of), though a
    /// saved setting naming it still resolves.
    #[serde(default)]
    pub hidden: bool,
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
    let coverage = cmap_table.map(cmap_coverage).unwrap_or_default();
    Some(FaceInfo {
        face_index,
        names,
        weight,
        italic,
        scripts: coverage.scripts,
        hidden: coverage.hidden,
    })
}

/// The `(platform id, encoding id)` of every subtable record of a raw `cmap` table.
fn cmap_encodings(data: &[u8]) -> Vec<(u16, u16)> {
    let be_u16 = |at: usize| {
        data.get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let count = be_u16(2).unwrap_or(0) as usize;
    (0..count)
        .map_while(|i| Some((be_u16(4 + 8 * i)?, be_u16(6 + 8 * i)?)))
        .collect()
}

/// What a raw `cmap` table covers: its Unicode subtables when it has any, else its legacy encodings (ADR 0042).
fn cmap_coverage(data: &[u8]) -> Coverage {
    let unicode: Vec<cmap::Subtable> = cmap::Table::parse(data)
        .map(|table| {
            let subtables = table.subtables;
            (0..subtables.len())
                .filter_map(|index| subtables.get(index))
                .filter(|subtable| subtable.is_unicode())
                .collect()
        })
        .unwrap_or_default();
    if unicode.is_empty() {
        return Coverage {
            scripts: legacy_encoding_scripts(&cmap_encodings(data)),
            hidden: false,
        };
    }
    let has_code = |code: u32| {
        unicode
            .iter()
            .any(|subtable| subtable.glyph_index(code).is_some_and(|g| g.0 != 0))
    };
    assess_coverage(
        |c| has_code(u32::from(c)),
        || {
            // Per subtable (a format-4 and a format-12 subtable list the same characters).
            let mut most = 0;
            for subtable in &unicode {
                let mut marks = 0;
                subtable.codepoints(|code| {
                    if is_cjk_mark(code)
                        && subtable.glyph_index(code).is_some_and(|glyph| glyph.0 != 0)
                    {
                        marks += 1;
                    }
                });
                most = most.max(marks);
            }
            most
        },
    )
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

    /// A `cmap` whose only subtable is a format-0 table under the legacy encoding `(platform, encoding)`.
    pub fn legacy_cmap_table(platform: u16, encoding: u16) -> Vec<u8> {
        let mut out = Vec::new();
        for value in [0u16, 1, platform, encoding] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(&legacy_subtable());
        out
    }

    /// A format-0 subtable mapping bytes 0x20 and up to glyph 1.
    fn legacy_subtable() -> Vec<u8> {
        let mut out = Vec::new();
        for value in [0u16, 262, 0] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend((0..=255u8).map(|byte| u8::from(byte >= 0x20)));
        out
    }

    /// Two subtables: `unicode` as in `cmap_table`, then a legacy one under `(platform, encoding)`.
    pub fn cmap_with_legacy(unicode: &str, platform: u16, encoding: u16) -> Vec<u8> {
        let unicode = cmap_table(unicode);
        let unicode_sub = &unicode[12..];
        let mut out = Vec::new();
        for value in [0u16, 2, 3, 10] {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.extend_from_slice(&20u32.to_be_bytes());
        out.extend_from_slice(&platform.to_be_bytes());
        out.extend_from_slice(&encoding.to_be_bytes());
        out.extend_from_slice(&(20 + unicode_sub.len() as u32).to_be_bytes());
        out.extend_from_slice(unicode_sub);
        out.extend_from_slice(&legacy_subtable());
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
    use crate::han_forms::{SIMPLIFIED_FORMS, TRADITIONAL_FORMS};
    use crate::sfnt::build_font;

    /// A Traditional Chinese font's characters: the common ones, the 200 Traditional forms and ㄅ.
    fn traditional_chars() -> String {
        format!(
            "的一是不了人我在有他這們中大來上國個到說時{}ㄅ",
            TRADITIONAL_FORMS.iter().collect::<String>()
        )
    }

    /// A Simplified Chinese font's characters: the common ones and the 200 Simplified forms.
    fn simplified_chars() -> String {
        format!(
            "的一是不了人我在有他这们中大来上国个到说时{}",
            SIMPLIFIED_FORMS.iter().collect::<String>()
        )
    }

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
        let cmap = cmap_table("Aaあいうえおのをアイウエオンヴ日本人一大年中出時行会見");
        let os2 = os2_table(700, true);
        let face = face_from_tables(0, &name, Some(&cmap), Some(&os2)).unwrap();
        assert_eq!((face.weight, face.italic), (700, true));
        assert_eq!(face.scripts, Scripts(Scripts::JAPANESE));
    }

    #[test]
    fn fonts_without_a_unicode_character_map_are_classified_by_their_encoding() {
        let name = name_table(&[(1, 0x0804, "长城粗圆体")]);
        let scripts = |cmap: Vec<u8>| {
            let face = face_from_tables(0, &name, Some(&cmap), None).unwrap();
            assert!(!face.hidden);
            face.scripts
        };
        assert_eq!(
            scripts(legacy_cmap_table(3, 3)),
            Scripts(Scripts::SIMPLIFIED)
        );
        assert_eq!(
            scripts(legacy_cmap_table(3, 4)),
            Scripts(Scripts::TRADITIONAL)
        );
        assert_eq!(scripts(legacy_cmap_table(3, 2)), Scripts(Scripts::JAPANESE));
        assert_eq!(scripts(legacy_cmap_table(3, 5)), Scripts(0), "Wansung");
        assert_eq!(
            scripts(legacy_cmap_table(1, 2)),
            Scripts(Scripts::TRADITIONAL)
        );
        // With a Unicode subtable the legacy one is ignored.
        assert_eq!(scripts(cmap_with_legacy("Aa", 3, 4)), Scripts(0));
        assert_eq!(
            scripts(cmap_with_legacy(&simplified_chars(), 3, 4)),
            Scripts(Scripts::SIMPLIFIED)
        );
    }

    #[test]
    fn faces_that_cannot_show_ordinary_text_are_hidden() {
        let name = name_table(&[(1, 0x0409, "Hidden?")]);
        let face =
            |chars: &str| face_from_tables(0, &name, Some(&cmap_table(chars)), None).unwrap();
        // Extension B only (細明體-ExtB).
        let ext_b: String = (0x20000..0x20040).filter_map(char::from_u32).collect();
        assert!(face(&format!("Aa{ext_b}")).hidden);
        // EUDC supplement (外字集): Bopomofo, a few common characters, rare ideographs, private-use glyphs.
        assert!(face("Aa的人ㄅㄆㄇㄈㄉㄊ丂丄丅丆丌丏丒丗丠丣\u{E000}\u{E001}").hidden);
        // Bopomofo letters and Latin (精靈文).
        assert!(
            face("Aaㄅㄆㄇㄈㄉㄊㄋㄌㄍㄎㄏㄐㄑㄒㄓㄔㄕㄖㄗㄘㄙㄚㄛㄜㄝㄞㄟㄠㄡㄢㄣㄤㄥㄦㄧㄨㄩ")
                .hidden
        );
        // A Latin font with a few stray CJK glyphs, private-use or compatibility-ideograph icons, plain Latin, Hangul
        // with a stray ideograph: listed.
        assert!(!face("Aa・卍卐\u{20000}").hidden);
        let icons: String = (0xF900..0xF940).filter_map(char::from_u32).collect();
        assert!(!face(&format!("Aa\u{E000}\u{F0000}{icons}")).hidden);
        assert!(!face("AaBbCc").hidden);
        assert!(!face("Aa가나다라마바사아자차카타파하一").hidden);
        let traditional = face(&traditional_chars());
        assert!(!traditional.hidden);
        assert_eq!(traditional.scripts, Scripts(Scripts::TRADITIONAL));
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
            face("Han TC", &traditional_chars()),
            face("Han SC", &simplified_chars()),
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
