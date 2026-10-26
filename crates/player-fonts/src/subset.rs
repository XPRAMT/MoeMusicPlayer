//! Name subsets (ADR 0042): a tiny font holding only the glyphs of one family's display name, so a picker row can be
//! drawn in its own font without the WebView loading the whole (often tens of MB) font file during layout.
//!
//! Only the face's outline and metric tables are read (with seeks, so a collection's other faces and the large
//! layout / bitmap tables stay on disk); `subsetter` (MIT OR Apache-2.0) cuts them down to the name's glyphs and this
//! module adds what a browser needs and `subsetter` leaves out for PDF use: a Unicode `cmap` for the kept characters,
//! the original `OS/2`, and a small `name` table.

use crate::sfnt::{self, TableRecord};
use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;

/// The version of what `name_subset` writes. A cached subset made by another version is regenerated.
pub const SUBSET_VERSION: u32 = 1;

/// Largest table copied into the standalone face (a CJK font's `glyf` is tens of MB).
const MAX_TABLE: u32 = 256 << 20;

/// The tables a name subset is made from; everything else (layout, bitmaps, vertical metrics, colour) stays on disk.
const SOURCE_TABLES: [&[u8; 4]; 14] = [
    b"head", b"hhea", b"hmtx", b"maxp", b"loca", b"glyf", b"CFF ", b"cvt ", b"fpgm", b"prep",
    b"name", b"post", b"cmap", b"OS/2",
];

/// The face `face_index` of the font file at `path` as a standalone sfnt holding only `SOURCE_TABLES`.
pub fn read_standalone_face(path: &Path, face_index: u32) -> io::Result<Vec<u8>> {
    let mut reader = BufReader::new(File::open(path)?);
    let offsets = sfnt::face_offsets(&mut reader)?;
    let offset = *offsets
        .get(face_index as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no such face"))?;
    let records = sfnt::table_records(&mut reader, offset)?;
    let mut tables = Vec::new();
    for tag in SOURCE_TABLES {
        if let Some(bytes) = sfnt::read_table(&mut reader, &records, tag, MAX_TABLE)? {
            tables.push((*tag, bytes));
        }
    }
    let flavor = if records.iter().any(|record| &record.tag == b"CFF ") {
        *b"OTTO"
    } else {
        [0, 1, 0, 0]
    };
    Ok(write_font(flavor, tables))
}

/// A font with only the glyphs of `text`'s characters from face `face_index` of `data` (a font file or collection),
/// or `None` when the face maps none of them through a Unicode `cmap` (a legacy-encoding font), lacks `OS/2` or
/// `name`, or cannot be subset (CFF2, damaged tables). Characters the face lacks are left out, so the browser draws
/// them in the next font of the stack.
pub fn name_subset(data: &[u8], face_index: u32, text: &str) -> Option<Vec<u8>> {
    let face = ttf_parser::Face::parse(data, face_index).ok()?;
    let raw = face.raw_face();
    let os2 = raw.table(ttf_parser::Tag::from_bytes(b"OS/2"))?;
    let name = raw.table(ttf_parser::Tag::from_bytes(b"name"))?;
    let mut wanted: Vec<(u32, u16)> = Vec::new();
    for character in text.chars() {
        if character.is_control() || wanted.iter().any(|&(code, _)| code == u32::from(character)) {
            continue;
        }
        if let Some(glyph) = face.glyph_index(character).filter(|glyph| glyph.0 != 0) {
            wanted.push((u32::from(character), glyph.0));
        }
    }
    if wanted.is_empty() {
        return None;
    }
    let mut remapper = subsetter::GlyphRemapper::new();
    for &(_, glyph) in &wanted {
        remapper.remap(glyph);
    }
    let subset = subsetter::subset(data, face_index, &remapper).ok()?;
    let mut mapping: Vec<(u32, u16)> = wanted
        .iter()
        .filter_map(|&(code, glyph)| Some((code, remapper.get(glyph)?)))
        .collect();
    mapping.sort_unstable();
    let (flavor, mut tables) = split_font(&subset)?;
    tables.retain(|(tag, _)| !matches!(tag, b"cmap" | b"OS/2" | b"name"));
    clear_reserved_glyph_flags(&mut tables);
    tables.push((*b"cmap", build_cmap(&mapping)));
    tables.push((*b"OS/2", os2.to_vec()));
    tables.push((
        *b"name",
        small_name_table(name).unwrap_or_else(|| name.to_vec()),
    ));
    Some(write_font(flavor, tables))
}

/// Clear the reserved bit 7 of every simple glyph's point flags. Some old system fonts (MingLiU) set it; the browser's
/// font sanitizer rejects a web font that does, while the same font installed in the system is drawn unchecked.
fn clear_reserved_glyph_flags(tables: &mut [([u8; 4], Vec<u8>)]) {
    let find =
        |tables: &[([u8; 4], Vec<u8>)], tag: &[u8; 4]| tables.iter().position(|(t, _)| t == tag);
    let (Some(head), Some(loca), Some(glyf)) = (
        find(tables, b"head"),
        find(tables, b"loca"),
        find(tables, b"glyf"),
    ) else {
        return;
    };
    let long = tables[head]
        .1
        .get(50..52)
        .is_some_and(|format| format == [0, 1]);
    let loca = &tables[loca].1;
    let offsets: Vec<usize> = if long {
        loca.chunks_exact(4)
            .map(|word| u32::from_be_bytes([word[0], word[1], word[2], word[3]]) as usize)
            .collect()
    } else {
        loca.chunks_exact(2)
            .map(|word| usize::from(u16::from_be_bytes([word[0], word[1]])) * 2)
            .collect()
    };
    let glyf = &mut tables[glyf].1;
    for range in offsets.windows(2) {
        let (start, end) = (range[0], range[1].min(glyf.len()));
        if start + 10 > end {
            continue;
        }
        let glyph = &mut glyf[start..end];
        let contours = i16::from_be_bytes([glyph[0], glyph[1]]);
        if contours <= 0 {
            continue; // empty or composite
        }
        let at = 10 + 2 * contours as usize;
        let Some(last) = glyph.get(at - 2..at) else {
            continue;
        };
        let points = usize::from(u16::from_be_bytes([last[0], last[1]])) + 1;
        let Some(instructions) = glyph.get(at..at + 2) else {
            continue;
        };
        let mut cursor =
            at + 2 + usize::from(u16::from_be_bytes([instructions[0], instructions[1]]));
        let mut seen = 0;
        while seen < points && cursor < glyph.len() {
            glyph[cursor] &= 0x7f;
            let repeat = glyph[cursor] & 0x08 != 0;
            cursor += 1;
            seen += 1;
            if repeat && cursor < glyph.len() {
                seen += usize::from(glyph[cursor]);
                cursor += 1;
            }
        }
    }
}

/// One table of a font being written: its tag and bytes.
type Table = ([u8; 4], Vec<u8>);

/// The sfnt version and tables of a plain (non-collection) font.
fn split_font(data: &[u8]) -> Option<([u8; 4], Vec<Table>)> {
    let mut reader = io::Cursor::new(data);
    let records: Vec<TableRecord> = sfnt::table_records(&mut reader, 0).ok()?;
    let flavor: [u8; 4] = data.get(0..4)?.try_into().ok()?;
    let tables = records
        .iter()
        .map(|record| {
            let start = record.offset as usize;
            let bytes = data.get(start..start.checked_add(record.length as usize)?)?;
            Some((record.tag, bytes.to_vec()))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((flavor, tables))
}

/// The face's Windows-platform names with IDs 0–6 (copyright, family, style, unique, full, version, PostScript), so a
/// subset is not burdened with long licence texts; `None` when it has none.
fn small_name_table(table: &[u8]) -> Option<Vec<u8>> {
    let parsed = ttf_parser::name::Table::parse(table)?;
    let records: Vec<_> = parsed
        .names
        .into_iter()
        .filter(|record| {
            record.platform_id == ttf_parser::PlatformId::Windows && record.name_id <= 6
        })
        .collect();
    if records.is_empty() {
        return None;
    }
    let header_len = 6 + 12 * records.len();
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&u16::try_from(records.len()).ok()?.to_be_bytes());
    out.extend_from_slice(&u16::try_from(header_len).ok()?.to_be_bytes());
    let mut storage: Vec<u8> = Vec::new();
    for record in &records {
        let offset = u16::try_from(storage.len()).ok()?;
        let length = u16::try_from(record.name.len()).ok()?;
        storage.extend_from_slice(record.name);
        out.extend_from_slice(&(record.platform_id as u16).to_be_bytes());
        out.extend_from_slice(&record.encoding_id.to_be_bytes());
        out.extend_from_slice(&record.language_id.to_be_bytes());
        out.extend_from_slice(&record.name_id.to_be_bytes());
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
    }
    out.extend_from_slice(&storage);
    Some(out)
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// A `cmap` for `mapping` (code point, glyph; sorted by code point): a format 4 subtable (Windows Unicode BMP) and,
/// when a character is outside the BMP, a format 12 subtable (Windows Unicode full repertoire).
pub(crate) fn build_cmap(mapping: &[(u32, u16)]) -> Vec<u8> {
    let bmp: Vec<(u16, u16)> = mapping
        .iter()
        .filter_map(|&(code, glyph)| {
            Some((u16::try_from(code).ok().filter(|&c| c != 0xFFFF)?, glyph))
        })
        .collect();
    // Format 4: runs of consecutive characters with consecutive glyphs share a segment, then the closing 0xFFFF.
    let mut segments: Vec<(u16, u16, u16)> = Vec::new(); // (start, end, delta)
    for &(code, glyph) in &bmp {
        let delta = glyph.wrapping_sub(code);
        match segments.last_mut() {
            Some((_, end, last_delta)) if *end + 1 == code && *last_delta == delta => *end = code,
            _ => segments.push((code, code, delta)),
        }
    }
    segments.push((0xFFFF, 0xFFFF, 1));
    let seg_count = segments.len() as u16;
    let entry_selector = 15 - seg_count.leading_zeros() as u16;
    let search_range = 2 * (1u16 << entry_selector);
    let mut format4 = Vec::new();
    push_u16(&mut format4, 4);
    push_u16(&mut format4, 16 + 8 * seg_count);
    push_u16(&mut format4, 0);
    push_u16(&mut format4, seg_count * 2);
    push_u16(&mut format4, search_range);
    push_u16(&mut format4, entry_selector);
    push_u16(&mut format4, seg_count * 2 - search_range);
    for &(_, end, _) in &segments {
        push_u16(&mut format4, end);
    }
    push_u16(&mut format4, 0);
    for &(start, _, _) in &segments {
        push_u16(&mut format4, start);
    }
    for &(_, _, delta) in &segments {
        push_u16(&mut format4, delta);
    }
    for _ in &segments {
        push_u16(&mut format4, 0);
    }
    let format12 = mapping.iter().any(|&(code, _)| code > 0xFFFF).then(|| {
        let mut groups: Vec<(u32, u32, u32)> = Vec::new();
        for &(code, glyph) in mapping {
            match groups.last_mut() {
                Some((start, end, first))
                    if *end + 1 == code && *first + (code - *start) == u32::from(glyph) =>
                {
                    *end = code
                }
                _ => groups.push((code, code, u32::from(glyph))),
            }
        }
        let mut table = Vec::new();
        push_u16(&mut table, 12);
        push_u16(&mut table, 0);
        push_u32(&mut table, 16 + 12 * groups.len() as u32);
        push_u32(&mut table, 0);
        push_u32(&mut table, groups.len() as u32);
        for (start, end, glyph) in groups {
            push_u32(&mut table, start);
            push_u32(&mut table, end);
            push_u32(&mut table, glyph);
        }
        table
    });
    let subtables: Vec<(u16, Vec<u8>)> = std::iter::once((1, format4))
        .chain(format12.map(|table| (10, table)))
        .collect();
    let mut out = Vec::new();
    push_u16(&mut out, 0);
    push_u16(&mut out, subtables.len() as u16);
    let mut offset = 4 + 8 * subtables.len() as u32;
    for (encoding, table) in &subtables {
        push_u16(&mut out, 3);
        push_u16(&mut out, *encoding);
        push_u32(&mut out, offset);
        offset += table.len() as u32;
    }
    for (_, table) in subtables {
        out.extend_from_slice(&table);
    }
    out
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// A plain sfnt with `tables` (sorted by tag, 4-byte aligned, checksums and the `head` checksum adjustment set).
pub(crate) fn write_font(flavor: [u8; 4], mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by(|a, b| a.0.cmp(&b.0));
    let count = tables.len() as u16;
    let entry_selector = if count == 0 {
        0
    } else {
        15 - count.leading_zeros() as u16
    };
    let search_range = (1u16 << entry_selector).saturating_mul(16);
    let mut out = Vec::new();
    out.extend_from_slice(&flavor);
    push_u16(&mut out, count);
    push_u16(&mut out, search_range);
    push_u16(&mut out, entry_selector);
    push_u16(&mut out, (count * 16).saturating_sub(search_range));
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, data) in &mut tables {
        if tag == b"head" && data.len() >= 12 {
            data[8..12].fill(0);
            head_at = Some(offset + 8);
        }
        out.extend_from_slice(tag);
        push_u32(&mut out, checksum(data));
        push_u32(&mut out, offset as u32);
        push_u32(&mut out, data.len() as u32);
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    if let Some(at) = head_at {
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at..at + 4].copy_from_slice(&adjustment.to_be_bytes());
    }
    out
}

/// A stable 64-bit FNV-1a hash as 16 hex digits: names cache entries and subset files without a hashing dependency.
pub fn stable_hash(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn be16(values: &[i32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|&v| (v as u16).to_be_bytes())
            .collect()
    }

    fn name_record_table(names: &[(u16, u16, &str)]) -> Vec<u8> {
        let mut storage = Vec::new();
        let mut out = be16(&[0, names.len() as i32, 6 + 12 * names.len() as i32]);
        for &(lang, id, text) in names {
            let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
            out.extend(be16(&[
                3,
                1,
                lang as i32,
                id as i32,
                bytes.len() as i32,
                storage.len() as i32,
            ]));
            storage.extend(bytes);
        }
        out.extend(storage);
        out
    }

    fn os2() -> Vec<u8> {
        let mut table = vec![0u8; 78];
        table[4..6].copy_from_slice(&400u16.to_be_bytes());
        table[6..8].copy_from_slice(&5u16.to_be_bytes());
        table
    }

    fn post() -> Vec<u8> {
        let mut table = vec![0u8; 32];
        table[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        table
    }

    fn head(index_to_loc: i32) -> Vec<u8> {
        let mut table = be16(&[1, 0, 1, 0]);
        table.extend([0; 4]); // checksum adjustment
        table.extend(0x5F0F_3CF5u32.to_be_bytes());
        table.extend(be16(&[0, 1000]));
        table.extend([0; 16]); // created, modified
        table.extend(be16(&[0, 0, 1000, 1000, 0, 8, 2, index_to_loc, 0]));
        table
    }

    fn hhea(metrics: usize) -> Vec<u8> {
        let mut table = be16(&[1, 0, 800, -200, 0, 1000, 0, 0, 1000, 1, 0, 0, 0, 0, 0, 0, 0]);
        table.extend(be16(&[metrics as i32]));
        table
    }

    /// Characters of the fixture fonts, each a square glyph (glyph i + 1 for character i).
    pub const FIXTURE_CHARS: [char; 4] = ['A', 'B', '字', '𠀀'];

    fn fixture_cmap() -> Vec<u8> {
        let mapping: Vec<(u32, u16)> = FIXTURE_CHARS
            .iter()
            .enumerate()
            .map(|(index, &c)| (u32::from(c), index as u16 + 1))
            .collect();
        build_cmap(&mapping)
    }

    fn common_tables(family: &str, glyphs: usize) -> Vec<([u8; 4], Vec<u8>)> {
        vec![
            (*b"hhea", hhea(glyphs)),
            (
                *b"hmtx",
                (0..glyphs).flat_map(|_| be16(&[1000, 0])).collect(),
            ),
            (*b"cmap", fixture_cmap()),
            (
                *b"name",
                name_record_table(&[
                    (0x0409, 0, "Copyright test"),
                    (0x0409, 1, family),
                    (0x0404, 1, "測試字型"),
                    (0x0409, 2, "Regular"),
                    (0x0409, 13, &"licence ".repeat(200)),
                ]),
            ),
            (*b"OS/2", os2()),
            (*b"post", post()),
        ]
    }

    /// A TrueType font: `.notdef` plus one square per `FIXTURE_CHARS` character.
    pub fn truetype_fixture(family: &str) -> Vec<([u8; 4], Vec<u8>)> {
        let glyphs = FIXTURE_CHARS.len() + 1;
        let square = |size: i32| {
            let mut glyph = be16(&[1, 0, 0, size, size, 3, 0]);
            glyph.extend([1u8; 4]); // four on-curve points, long coordinates
            glyph.extend(be16(&[0, size, 0, -size])); // x deltas
            glyph.extend(be16(&[0, 0, size, 0])); // y deltas
            glyph
        };
        let mut glyf = Vec::new();
        for index in 1..glyphs {
            glyf.extend(square(100 * index as i32));
        }
        let mut tables = common_tables(family, glyphs);
        tables.push((*b"head", head(0)));
        let mut maxp = be16(&[1, 0, glyphs as i32, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0]);
        maxp.truncate(32);
        tables.push((*b"maxp", maxp));
        // Glyph 0 is empty: loca starts 0, 0, then each square's end.
        let mut offsets = vec![0, 0];
        let mut end = 0;
        for index in 1..glyphs {
            end += square(100 * index as i32).len() as i32 / 2;
            offsets.push(end);
        }
        tables.push((*b"loca", be16(&offsets)));
        tables.push((*b"glyf", glyf));
        tables.sort_by(|a, b| a.0.cmp(&b.0));
        tables
    }

    fn cff_index(items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = be16(&[items.len() as i32]);
        if items.is_empty() {
            return out;
        }
        out.push(2); // offSize
        let mut offset = 1;
        out.extend(be16(&[offset]));
        for item in items {
            offset += item.len() as i32;
            out.extend(be16(&[offset]));
        }
        for item in items {
            out.extend(item);
        }
        out
    }

    fn cff_int(value: i32) -> Vec<u8> {
        let mut out = vec![29];
        out.extend(value.to_be_bytes());
        out
    }

    fn charstring_number(value: i32) -> Vec<u8> {
        let mut out = vec![28];
        out.extend((value as i16).to_be_bytes());
        out
    }

    /// A CFF (OpenType `OTTO`) font with the same glyphs as `truetype_fixture`.
    pub fn cff_fixture(family: &str) -> Vec<([u8; 4], Vec<u8>)> {
        let glyphs = FIXTURE_CHARS.len() + 1;
        let mut charstrings = vec![vec![14u8]];
        for index in 1..glyphs {
            let size = 100 * index as i32;
            let mut program = Vec::new();
            for value in [0, 0] {
                program.extend(charstring_number(value));
            }
            program.push(21); // rmoveto
            for value in [size, 0, 0, size, -size, 0] {
                program.extend(charstring_number(value));
            }
            program.push(5); // rlineto
            program.push(14); // endchar
            charstrings.push(program);
        }
        let name_index = cff_index(&[b"TestCFF".to_vec()]);
        let string_index = cff_index(&[]);
        let global_subrs = cff_index(&[]);
        let charstrings_index = cff_index(&charstrings);
        let private = vec![139u8, 20]; // defaultWidthX 0
                                       // The top DICT has a fixed size (5-byte integers), so the offsets can be computed before it is written.
        let top_len = (5 + 1) + (5 + 5 + 1);
        let top_index_len = cff_index(&[vec![0; top_len]]).len();
        let charstrings_at =
            4 + name_index.len() + top_index_len + string_index.len() + global_subrs.len();
        let private_at = charstrings_at + charstrings_index.len();
        let mut top = cff_int(charstrings_at as i32);
        top.push(17);
        top.extend(cff_int(private.len() as i32));
        top.extend(cff_int(private_at as i32));
        top.push(18);
        assert_eq!(top.len(), top_len);
        let mut cff = vec![1, 0, 4, 2];
        cff.extend(name_index);
        cff.extend(cff_index(&[top]));
        cff.extend(string_index);
        cff.extend(global_subrs);
        cff.extend(charstrings_index);
        cff.extend(private);
        let mut tables = common_tables(family, glyphs);
        tables.push((*b"head", head(0)));
        tables.push((*b"maxp", {
            let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
            maxp.extend(be16(&[glyphs as i32]));
            maxp
        }));
        tables.push((*b"CFF ", cff));
        tables.sort_by(|a, b| a.0.cmp(&b.0));
        tables
    }

    fn assert_subset_draws(subset: &[u8], text: &str) {
        let face = ttf_parser::Face::parse(subset, 0).expect("the subset parses");
        for character in text.chars() {
            let glyph = face
                .glyph_index(character)
                .unwrap_or_else(|| panic!("{character} is mapped"));
            let mut sink = Sink(0);
            let bbox = face.outline_glyph(glyph, &mut sink).expect("an outline");
            assert!(bbox.width() > 0 && sink.0 > 0, "{character} has its square");
        }
        let raw = face.raw_face();
        for tag in [
            b"cmap", b"OS/2", b"name", b"head", b"hhea", b"hmtx", b"maxp", b"post",
        ] {
            assert!(
                raw.table(ttf_parser::Tag::from_bytes(tag)).is_some(),
                "{}",
                String::from_utf8_lossy(tag)
            );
        }
        let names = ttf_parser::name::Table::parse(
            raw.table(ttf_parser::Tag::from_bytes(b"name")).unwrap(),
        )
        .unwrap();
        assert!(
            names.names.into_iter().all(|record| record.name_id <= 6),
            "the licence text is left out"
        );
    }

    struct Sink(usize);
    impl ttf_parser::OutlineBuilder for Sink {
        fn move_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn line_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }

    fn glyph_count(font: &[u8]) -> u16 {
        ttf_parser::Face::parse(font, 0).unwrap().number_of_glyphs()
    }

    #[test]
    fn a_truetype_subset_keeps_only_the_name_glyphs() {
        let font = sfnt::build_font(&[truetype_fixture("Test Sans")]);
        assert!(
            ttf_parser::Face::parse(&font, 0).is_ok(),
            "the fixture is a valid font"
        );
        let subset = name_subset(&font, 0, "字A字").expect("a subset");
        assert_subset_draws(&subset, "字A");
        assert_eq!(glyph_count(&subset), 3, ".notdef and the two characters");
        let face = ttf_parser::Face::parse(&subset, 0).unwrap();
        assert_eq!(
            face.glyph_index('B'),
            None,
            "characters not in the name are left out"
        );
        // A character outside the BMP goes through a format 12 subtable.
        let wide = name_subset(&font, 0, "𠀀B").expect("a subset");
        assert_subset_draws(&wide, "𠀀B");
    }

    #[test]
    fn a_cff_subset_keeps_only_the_name_glyphs() {
        let font = sfnt::build_font(&[cff_fixture("Test Serif")]);
        let font = {
            let mut font = font;
            font[0..4].copy_from_slice(b"OTTO");
            font
        };
        assert!(
            ttf_parser::Face::parse(&font, 0).is_ok(),
            "the fixture is a valid font"
        );
        let subset = name_subset(&font, 0, "BA").expect("a subset");
        assert_eq!(&subset[0..4], b"OTTO");
        assert_subset_draws(&subset, "AB");
        assert_eq!(glyph_count(&subset), 3);
    }

    #[test]
    fn a_collection_face_is_read_on_its_own() {
        let collection = sfnt::build_font(&[truetype_fixture("First"), truetype_fixture("Second")]);
        let dir = std::env::temp_dir().join(format!("moe-subset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pair.ttc");
        std::fs::write(&path, &collection).unwrap();
        let second = read_standalone_face(&path, 1).expect("face 1");
        let family = ttf_parser::Face::parse(&second, 0)
            .unwrap()
            .names()
            .into_iter()
            .filter(|name| name.name_id == 1 && name.language_id == 0x0409)
            .find_map(|name| name.to_string());
        assert_eq!(family.as_deref(), Some("Second"));
        let subset = name_subset(&second, 0, "AB").expect("a subset");
        assert_subset_draws(&subset, "AB");
        // The same face straight from the collection data.
        assert_subset_draws(&name_subset(&collection, 1, "AB").expect("a subset"), "AB");
        assert!(read_standalone_face(&path, 2).is_err(), "no third face");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_font_without_the_characters_or_a_unicode_cmap_has_no_subset() {
        let font = sfnt::build_font(&[truetype_fixture("Test Sans")]);
        assert_eq!(name_subset(&font, 0, "あ"), None, "none of the characters");
        // A legacy-encoding font: only a Windows Big5 (3,4) character map.
        let mut tables = truetype_fixture("Legacy");
        for (tag, data) in &mut tables {
            if tag == b"cmap" {
                // Both encoding records (format 4 and format 12) become Windows Big5.
                data[6..8].copy_from_slice(&4u16.to_be_bytes());
                data[14..16].copy_from_slice(&4u16.to_be_bytes());
            }
        }
        let legacy = sfnt::build_font(&[tables]);
        assert_eq!(name_subset(&legacy, 0, "AB"), None);
        assert_eq!(name_subset(b"not a font", 0, "AB"), None);
    }

    #[test]
    fn reserved_glyph_flags_are_cleared() {
        let mut tables = truetype_fixture("Old Ming");
        for (tag, data) in &mut tables {
            if tag == b"glyf" {
                // Glyph 1 (A): its first flag gets the reserved bit, as in MingLiU.
                data[14] |= 0x80;
            }
        }
        let font = sfnt::build_font(&[tables]);
        let subset = name_subset(&font, 0, "A").expect("a subset");
        let (_, tables) = split_font(&subset).unwrap();
        let glyf = &tables.iter().find(|(tag, _)| tag == b"glyf").unwrap().1;
        assert!(
            glyf.iter().skip(14).take(4).all(|&flag| flag == 1),
            "{glyf:?}"
        );
        assert_subset_draws(&subset, "A");
    }

    #[test]
    fn the_cmap_merges_runs_and_closes_with_ffff() {
        let cmap = build_cmap(&[(0x41, 1), (0x42, 2), (0x5B57, 3)]);
        let table = ttf_parser::cmap::Table::parse(&cmap).unwrap();
        let subtable = table.subtables.get(0).unwrap();
        assert!(subtable.is_unicode());
        assert_eq!(subtable.glyph_index(0x41).map(|g| g.0), Some(1));
        assert_eq!(subtable.glyph_index(0x42).map(|g| g.0), Some(2));
        assert_eq!(subtable.glyph_index(0x5B57).map(|g| g.0), Some(3));
        assert_eq!(subtable.glyph_index(0x43), None);
        // Two segments (A–B, 字) plus 0xFFFF.
        assert_eq!(u16::from_be_bytes([cmap[12 + 6], cmap[12 + 7]]), 6);
        assert_eq!(
            table.subtables.len(),
            1,
            "no format 12 without supplementary characters"
        );
    }

    #[test]
    fn hashes_are_stable_and_separate_parts() {
        assert_eq!(stable_hash(&["a", "bc"]), stable_hash(&["a", "bc"]));
        assert_ne!(stable_hash(&["ab", "c"]), stable_hash(&["a", "bc"]));
        assert_eq!(stable_hash(&[]).len(), 16);
    }
}
