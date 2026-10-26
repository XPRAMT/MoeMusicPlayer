//! The sfnt / TrueType Collection table directory, read with seeks so only the few tables we need (`name`, `cmap`,
//! `OS/2`) are loaded: a CJK font file can be tens of MB.

use std::io::{self, Read, Seek, SeekFrom};

/// Collections with more faces than this are treated as damaged.
const MAX_FACES: u32 = 256;
/// Faces with more tables than this are treated as damaged.
const MAX_TABLES: u16 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableRecord {
    pub tag: [u8; 4],
    pub offset: u32,
    pub length: u32,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn be_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Offsets of the faces' table directories: one for a plain sfnt, one per face of a TrueType / OpenType Collection.
pub fn face_offsets<R: Read + Seek>(reader: &mut R) -> io::Result<Vec<u32>> {
    reader.seek(SeekFrom::Start(0))?;
    let mut header = [0u8; 12];
    reader.read_exact(&mut header)?;
    match &header[0..4] {
        b"ttcf" => {
            let count = be_u32(&header[8..12]);
            if count == 0 || count > MAX_FACES {
                return Err(invalid("bad collection face count"));
            }
            let mut offsets = vec![0u8; count as usize * 4];
            reader.read_exact(&mut offsets)?;
            Ok(offsets.chunks_exact(4).map(be_u32).collect())
        }
        [0x00, 0x01, 0x00, 0x00] | b"OTTO" | b"true" => Ok(vec![0]),
        _ => Err(invalid("not an sfnt font")),
    }
}

/// The table records of the face whose directory starts at `offset`.
pub fn table_records<R: Read + Seek>(reader: &mut R, offset: u32) -> io::Result<Vec<TableRecord>> {
    reader.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut header = [0u8; 12];
    reader.read_exact(&mut header)?;
    let count = be_u16(&header[4..6]);
    if count > MAX_TABLES {
        return Err(invalid("bad table count"));
    }
    let mut records = vec![0u8; count as usize * 16];
    reader.read_exact(&mut records)?;
    Ok(records
        .chunks_exact(16)
        .map(|record| TableRecord {
            tag: [record[0], record[1], record[2], record[3]],
            offset: be_u32(&record[8..12]),
            length: be_u32(&record[12..16]),
        })
        .collect())
}

/// The bytes of table `tag`, if the face has it and it is at most `max_len` long.
pub fn read_table<R: Read + Seek>(
    reader: &mut R,
    records: &[TableRecord],
    tag: &[u8; 4],
    max_len: u32,
) -> io::Result<Option<Vec<u8>>> {
    let Some(record) = records.iter().find(|record| &record.tag == tag) else {
        return Ok(None);
    };
    if record.length > max_len {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(u64::from(record.offset)))?;
    let mut bytes = vec![0u8; record.length as usize];
    reader.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

/// Test helper: an sfnt file (or, with several faces, a collection) holding the given tables.
#[cfg(any(test, feature = "testing"))]
pub fn build_font(faces: &[Vec<([u8; 4], Vec<u8>)>]) -> Vec<u8> {
    let collection = faces.len() > 1;
    let header_len = if collection { 12 + 4 * faces.len() } else { 0 };
    let mut directories: Vec<Vec<u8>> = Vec::new();
    let mut blobs: Vec<u8> = Vec::new();
    let directory_len: usize = faces.iter().map(|tables| 12 + 16 * tables.len()).sum();
    let data_start = header_len + directory_len;
    for tables in faces {
        let mut directory = Vec::new();
        directory.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        directory.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        directory.extend_from_slice(&[0; 6]);
        for (tag, data) in tables {
            let offset = data_start + blobs.len();
            directory.extend_from_slice(tag);
            directory.extend_from_slice(&[0; 4]);
            directory.extend_from_slice(&(offset as u32).to_be_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_be_bytes());
            blobs.extend_from_slice(data);
            while !blobs.len().is_multiple_of(4) {
                blobs.push(0);
            }
        }
        directories.push(directory);
    }
    let mut out = Vec::new();
    if collection {
        out.extend_from_slice(b"ttcf");
        out.extend_from_slice(&[0, 1, 0, 0]);
        out.extend_from_slice(&(faces.len() as u32).to_be_bytes());
        let mut offset = header_len;
        for directory in &directories {
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            offset += directory.len();
        }
    }
    for directory in &directories {
        out.extend_from_slice(directory);
    }
    out.extend_from_slice(&blobs);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_plain_and_collection_directories() {
        let plain = build_font(&[vec![(*b"name", vec![1, 2, 3]), (*b"cmap", vec![4, 5])]]);
        let mut reader = Cursor::new(plain);
        assert_eq!(face_offsets(&mut reader).unwrap(), vec![0]);
        let records = table_records(&mut reader, 0).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(
            read_table(&mut reader, &records, b"cmap", 100).unwrap(),
            Some(vec![4, 5])
        );
        assert_eq!(
            read_table(&mut reader, &records, b"OS/2", 100).unwrap(),
            None
        );
        assert_eq!(
            read_table(&mut reader, &records, b"name", 2).unwrap(),
            None,
            "over the size cap"
        );

        let collection = build_font(&[vec![(*b"name", vec![9])], vec![(*b"name", vec![8, 7])]]);
        let mut reader = Cursor::new(collection);
        let offsets = face_offsets(&mut reader).unwrap();
        assert_eq!(offsets.len(), 2);
        let second = table_records(&mut reader, offsets[1]).unwrap();
        assert_eq!(
            read_table(&mut reader, &second, b"name", 100).unwrap(),
            Some(vec![8, 7])
        );
    }

    #[test]
    fn rejects_files_that_are_not_fonts() {
        let mut reader = Cursor::new(b"wOFF\0\0\0\0\0\0\0\0".to_vec());
        assert!(face_offsets(&mut reader).is_err());
        let mut reader = Cursor::new(b"ab".to_vec());
        assert!(face_offsets(&mut reader).is_err());
    }
}
