//! A cover is read from a song on a server with ranged reads of the tag only: a 28 MB song with a 300 KB cover asks the server
//! for about the size of the tag, whatever the container, and a lookup that was cancelled asks for nothing more.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use player_remote::fs::{fetched_bytes_on_thread, metadata_reader, RemoteError, RemoteFile};
use player_tags::embedded::read_embedded_tags;

/// A file on a "server" that records every request made to it.
struct Served {
    data: Arc<Vec<u8>>,
    requests: Arc<Mutex<Vec<(u64, u64)>>>,
    count: Arc<AtomicU64>,
}

impl RemoteFile for Served {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        let start = (offset as usize).min(self.data.len());
        let count = buf.len().min(self.data.len() - start);
        buf[..count].copy_from_slice(&self.data[start..start + count]);
        self.requests
            .lock()
            .unwrap()
            .push((offset, offset + count as u64));
        self.count.fetch_add(1, Ordering::SeqCst);
        Ok(count)
    }
}

type Requests = Arc<Mutex<Vec<(u64, u64)>>>;

fn served(bytes: Vec<u8>) -> (Served, Requests) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    (
        Served {
            data: Arc::new(bytes),
            requests: Arc::clone(&requests),
            count: Arc::new(AtomicU64::new(0)),
        },
        requests,
    )
}

const AUDIO: usize = 28_000_000;
const COVER: usize = 300_000;

fn cover_bytes() -> Vec<u8> {
    let mut data = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    data.resize(COVER, 0x5a);
    data
}

fn synchsafe(value: u32) -> [u8; 4] {
    [
        ((value >> 21) & 0x7f) as u8,
        ((value >> 14) & 0x7f) as u8,
        ((value >> 7) & 0x7f) as u8,
        (value & 0x7f) as u8,
    ]
}

fn mp3(cover_first: bool) -> Vec<u8> {
    let mut apic = vec![0];
    apic.extend_from_slice(b"image/png\0");
    apic.push(3);
    apic.push(0);
    apic.extend_from_slice(&cover_bytes());
    let mut tag = b"APIC".to_vec();
    tag.extend_from_slice(&synchsafe(apic.len() as u32));
    tag.extend_from_slice(&[0, 0]);
    tag.extend_from_slice(&apic);
    let mut file = b"ID3\x04\0\0".to_vec();
    file.extend_from_slice(&synchsafe(tag.len() as u32));
    file.extend_from_slice(&tag);
    let _ = cover_first;
    // MPEG-1 layer III 128 kbit/s 44.1 kHz frames: 417 bytes each.
    while file.len() < AUDIO {
        file.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        file.resize(file.len() + 413, 0);
    }
    file
}

fn flac() -> Vec<u8> {
    let mut file = b"fLaC".to_vec();
    // STREAMINFO (not last), 34 bytes: 44.1 kHz, 2 channels, 16 bit.
    file.extend_from_slice(&[0, 0, 0, 34]);
    let mut info = vec![0u8; 34];
    info[0..2].copy_from_slice(&4096u16.to_be_bytes());
    info[2..4].copy_from_slice(&4096u16.to_be_bytes());
    info[10] = 0x0a;
    info[11] = 0xc4;
    info[12] = 0x42;
    info[13] = 0xf0;
    file.extend_from_slice(&info);
    // PICTURE (last).
    let mut picture = Vec::new();
    picture.extend_from_slice(&3u32.to_be_bytes());
    picture.extend_from_slice(&9u32.to_be_bytes());
    picture.extend_from_slice(b"image/png");
    picture.extend_from_slice(&0u32.to_be_bytes());
    picture.extend_from_slice(&[0; 16]);
    picture.extend_from_slice(&(COVER as u32).to_be_bytes());
    picture.extend_from_slice(&cover_bytes());
    file.push(0x80 | 6);
    file.extend_from_slice(&(picture.len() as u32).to_be_bytes()[1..]);
    file.extend_from_slice(&picture);
    file.resize(AUDIO, 0xaa);
    file
}

fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn m4a(moov_first: bool) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&((cover_bytes().len() + 16) as u32).to_be_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&14u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&cover_bytes());
    let covr = atom(b"covr", &data);
    let ilst = atom(b"ilst", &covr);
    let mut meta_body = vec![0, 0, 0, 0];
    meta_body.extend_from_slice(&atom(b"hdlr", &[0; 25]));
    meta_body.extend_from_slice(&ilst);
    let meta = atom(b"meta", &meta_body);
    let udta = atom(b"udta", &meta);
    let moov = atom(b"moov", &udta);
    let ftyp = atom(b"ftyp", b"M4A \0\0\0\0M4A mp42isom");
    let mdat = atom(b"mdat", &vec![0x11; AUDIO]);
    let mut file = ftyp;
    if moov_first {
        file.extend_from_slice(&moov);
        file.extend_from_slice(&mdat);
    } else {
        file.extend_from_slice(&mdat);
        file.extend_from_slice(&moov);
    }
    file
}

fn dsf() -> Vec<u8> {
    let payload = vec![0x69u8; AUDIO / 2];
    let mut file =
        player_dsd::testing::build_dsf(2_822_400, 80_000, 4096, true, &[payload.clone(), payload]);
    let offset = file.len() as u64;
    file[20..28].copy_from_slice(&offset.to_le_bytes());
    let mut apic = vec![0];
    apic.extend_from_slice(b"image/png\0");
    apic.push(3);
    apic.push(0);
    apic.extend_from_slice(&cover_bytes());
    let mut tag = b"APIC".to_vec();
    tag.extend_from_slice(&synchsafe(apic.len() as u32));
    tag.extend_from_slice(&[0, 0]);
    tag.extend_from_slice(&apic);
    file.extend_from_slice(b"ID3\x04\0\0");
    file.extend_from_slice(&synchsafe(tag.len() as u32));
    file.extend_from_slice(&tag);
    file
}

/// The cover (its size) and the bytes the server was asked for.
fn lookup(bytes: Vec<u8>, name: &str) -> (Option<usize>, u64) {
    let (file, requests) = served(bytes);
    let before = fetched_bytes_on_thread();
    let reader = metadata_reader(Box::new(file));
    let cover = read_embedded_tags(reader, Path::new(name))
        .and_then(|tags| tags.front_cover().map(|picture| picture.data().len()));
    let asked: u64 = requests.lock().unwrap().iter().map(|(s, e)| e - s).sum();
    assert_eq!(
        asked,
        fetched_bytes_on_thread() - before,
        "the thread counter is what the server was asked for"
    );
    eprintln!("{name}: cover {cover:?}, {asked} bytes asked of the server");
    (cover, asked)
}

#[test]
fn the_cover_of_a_28_mb_song_on_a_server_costs_about_the_size_of_its_tag() {
    for (bytes, name) in [
        (mp3(true), "a.mp3"),
        (flac(), "a.flac"),
        (m4a(false), "moov-last.m4a"),
        (m4a(true), "moov-first.m4a"),
        (dsf(), "a.dsf"),
    ] {
        let (cover, asked) = lookup(bytes, name);
        assert_eq!(cover, Some(COVER), "{name}: the cover was found");
        // The tag, the blocks it is read in (16 KiB), the growing read-ahead of a run (at most 1 MiB past what is read) and
        // the few bytes of the end of the file the tag probes look at.
        assert!(
            asked <= COVER as u64 + 1_100_000,
            "{name}: the server was asked for {asked} bytes for a {COVER} byte cover"
        );
        assert!(
            asked < 28_000_000 / 10,
            "{name}: nowhere near the whole song"
        );
    }
}

#[test]
fn a_song_without_a_cover_asks_the_server_for_a_few_kilobytes() {
    let mut file = flac();
    file[4] |= 0x80;
    let (cover, asked) = lookup(file, "none.flac");
    assert_eq!(cover, None);
    assert!(asked <= 64 * 1024, "{asked} bytes");
}

#[test]
fn a_cancelled_lookup_asks_the_server_for_nothing_more() {
    let (file, _) = served(flac());
    let count = Arc::clone(&file.count);
    let flag = Arc::new(AtomicBool::new(false));
    let mut reader = metadata_reader(Box::new(file));
    let mut buffer = vec![0u8; 4096];
    player_core::with_read_cancel(Arc::clone(&flag), || {
        reader.read_exact(&mut buffer).unwrap();
        let after_first = count.load(Ordering::SeqCst);
        assert!(after_first > 0);
        flag.store(true, Ordering::SeqCst);
        // What is in the reader's memory is still answered; the next block is not asked for.
        reader.seek(SeekFrom::Start(10_000_000)).unwrap();
        let error = reader.read(&mut buffer).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(count.load(Ordering::SeqCst), after_first);
        // The tag parser sees the cancellation as a lookup that found nothing, and asks for nothing either.
        let (file, _) = served(flac());
        let count = Arc::clone(&file.count);
        let cover = read_embedded_tags(metadata_reader(Box::new(file)), Path::new("a.flac"));
        assert!(cover.is_none_or(|tags| tags.front_cover().is_none()));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    });
}
