//! Minimal Ogg reading: a page-header index of the first logical stream and a sequential packet reader.
//!
//! Page layout: `OggS`, version, header flags (1 = continued packet, 2 = first page, 4 = last page), granule
//! position (i64, −1 when no packet ends on the page), serial, sequence, CRC, segment count, segment table, body.
//! Segments of 255 bytes continue a packet; a shorter segment ends it. CRCs are not verified.

use std::collections::VecDeque;
use std::io::{BufReader, Read, Seek, SeekFrom};

use crate::OpusError;

const PAGE_HEADER_LEN: usize = 27;
const FLAG_CONTINUED: u8 = 0x01;
const FLAG_LAST: u8 = 0x04;
/// Largest packet accepted (OpusTags with cover art can be several megabytes).
const MAX_PACKET_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct PageHeader {
    pub(crate) flags: u8,
    pub(crate) granule: i64,
    pub(crate) serial: u32,
    pub(crate) segments: Vec<u8>,
}

impl PageHeader {
    pub(crate) fn continued(&self) -> bool {
        self.flags & FLAG_CONTINUED != 0
    }

    pub(crate) fn last(&self) -> bool {
        self.flags & FLAG_LAST != 0
    }

    pub(crate) fn body_len(&self) -> usize {
        self.segments.iter().map(|segment| usize::from(*segment)).sum()
    }
}

/// Read the header at the current position. `Ok(None)` is a clean end of file.
pub(crate) fn read_page_header<R: Read>(reader: &mut R) -> Result<Option<PageHeader>, OpusError> {
    let mut fixed = [0_u8; PAGE_HEADER_LEN];
    let mut filled = 0;
    while filled < PAGE_HEADER_LEN {
        let read = reader.read(&mut fixed[filled..])?;
        if read == 0 {
            return if filled == 0 {
                Ok(None)
            } else {
                Err(OpusError::InvalidFormat("Ogg 頁面標頭被截斷".into()))
            };
        }
        filled += read;
    }
    if &fixed[..4] != b"OggS" {
        return Err(OpusError::InvalidFormat("不是 Ogg 頁面（缺少 OggS）".into()));
    }
    if fixed[4] != 0 {
        return Err(OpusError::InvalidFormat("不支援的 Ogg 版本".into()));
    }
    let mut segments = vec![0_u8; usize::from(fixed[26])];
    reader.read_exact(&mut segments)?;
    Ok(Some(PageHeader {
        flags: fixed[5],
        granule: i64::from_le_bytes(fixed[6..14].try_into().expect("8 bytes")),
        serial: u32::from_le_bytes(fixed[14..18].try_into().expect("4 bytes")),
        segments,
    }))
}

/// One page of the first logical stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageEntry {
    pub(crate) offset: u64,
    pub(crate) granule: i64,
    pub(crate) continued: bool,
}

/// Index every page of the first logical stream (header and body bytes are skipped, not read). Stops after the
/// stream's last page, so a chained second stream is ignored.
pub(crate) fn scan_pages<R: Read + Seek>(reader: R) -> Result<Vec<PageEntry>, OpusError> {
    let mut reader = BufReader::with_capacity(64 * 1024, reader);
    reader.seek(SeekFrom::Start(0))?;
    let mut entries = Vec::new();
    let mut serial = None;
    let mut offset = 0_u64;
    while let Some(header) = read_page_header(&mut reader)? {
        let header_len = (PAGE_HEADER_LEN + header.segments.len()) as u64;
        let body = header.body_len() as u64;
        let first = *serial.get_or_insert(header.serial);
        if header.serial == first {
            entries.push(PageEntry {
                offset,
                granule: header.granule,
                continued: header.continued(),
            });
            if header.last() {
                break;
            }
        }
        reader.seek_relative(body as i64)?;
        offset += header_len + body;
    }
    Ok(entries)
}

/// Sequential packet reader over the pages of one logical stream.
pub(crate) struct PacketReader<R: Read + Seek> {
    reader: BufReader<R>,
    serial: Option<u32>,
    /// Bytes of a packet that continues on the next page.
    pending: Vec<u8>,
    queue: VecDeque<Vec<u8>>,
    finished: bool,
    /// File offset of the page the next `read_page` will read.
    next_page_offset: u64,
}

impl<R: Read + Seek> PacketReader<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader: BufReader::with_capacity(64 * 1024, reader),
            serial: None,
            pending: Vec::new(),
            queue: VecDeque::new(),
            finished: false,
            next_page_offset: 0,
        }
    }

    /// Continue reading from the page at `offset` (which must not be a continuation page).
    pub(crate) fn seek_to_page(&mut self, offset: u64) -> Result<(), OpusError> {
        self.reader.seek(SeekFrom::Start(offset))?;
        self.pending.clear();
        self.queue.clear();
        self.finished = false;
        self.next_page_offset = offset;
        Ok(())
    }

    pub(crate) fn next_page_offset(&self) -> u64 {
        self.next_page_offset
    }

    /// The next whole packet, or `None` at the end of the stream.
    pub(crate) fn next_packet(&mut self) -> Result<Option<Vec<u8>>, OpusError> {
        loop {
            if let Some(packet) = self.queue.pop_front() {
                return Ok(Some(packet));
            }
            if self.finished {
                return Ok(None);
            }
            self.read_page()?;
        }
    }

    fn read_page(&mut self) -> Result<(), OpusError> {
        let Some(header) = read_page_header(&mut self.reader)? else {
            self.finished = true;
            return Ok(());
        };
        let header_len = (PAGE_HEADER_LEN + header.segments.len()) as u64;
        let mut body = vec![0_u8; header.body_len()];
        self.reader.read_exact(&mut body)?;
        self.next_page_offset += header_len + body.len() as u64;
        match self.serial {
            None => self.serial = Some(header.serial),
            Some(serial) if serial != header.serial => return Ok(()),
            Some(_) => {}
        }
        if !header.continued() {
            // A packet cut short by a lost page is dropped.
            self.pending.clear();
        }
        let mut cursor = 0;
        for &segment in &header.segments {
            let length = usize::from(segment);
            self.pending.extend_from_slice(&body[cursor..cursor + length]);
            cursor += length;
            if self.pending.len() > MAX_PACKET_BYTES {
                return Err(OpusError::InvalidFormat("Ogg 封包過大".into()));
            }
            if segment < 255 {
                self.queue.push_back(std::mem::take(&mut self.pending));
            }
        }
        if header.last() {
            self.finished = true;
        }
        Ok(())
    }
}
