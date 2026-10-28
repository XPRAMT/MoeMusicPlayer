//! Reading the cover of a track reads only the tag, never the audio (a 28 MB song with a cover costs the size of the tag,
//! local or remote alike).

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

use player_tags::embedded::read_embedded_tags;

/// A reader over bytes that records every range handed out, so a test can bound what a lookup read.
struct Counting {
    inner: Cursor<Vec<u8>>,
    ranges: Arc<Mutex<Vec<(u64, u64)>>>,
}

impl Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let start = self.inner.position();
        let count = self.inner.read(buf)?;
        if count > 0 {
            self.ranges.lock().unwrap().push((start, start + count as u64));
        }
        Ok(count)
    }
}

impl Seek for Counting {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// (distinct bytes read, the lowest offset read past `head`, the highest offset read) of a lookup.
fn read_cover(bytes: Vec<u8>, name: &str) -> (Option<usize>, u64, u64) {
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let reader = Counting {
        inner: Cursor::new(bytes),
        ranges: Arc::clone(&ranges),
    };
    let cover = read_embedded_tags(reader, Path::new(name))
        .and_then(|tags| tags.front_cover().map(|picture| picture.data().len()));
    let mut ranges = ranges.lock().unwrap().clone();
    ranges.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let distinct: u64 = merged.iter().map(|(s, e)| e - s).sum();
    let highest = merged.last().map_or(0, |r| r.1);
    eprintln!("{name}: cover {cover:?}, read {distinct} bytes in {} runs {:?}", merged.len(), &merged[..merged.len().min(6)]);
    (cover, distinct, highest)
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
    let mut file = player_dsd::testing::build_dsf(2_822_400, 80_000, 4096, true, &[payload.clone(), payload]);
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

/// A WavPack file whose audio is `AUDIO` bytes long, with its APEv2 cover tag at the very end.
fn wavpack_with_tail_tag() -> Vec<u8> {
    use player_tags::{write_edit, TagEdit};
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../player-wavpack/tests/fixtures/s16.wv");
    let dir = std::env::temp_dir().join(format!("moe-read-bounds-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.wv");
    std::fs::copy(fixture, &path).unwrap();
    let mut edit = TagEdit::default();
    edit.fields.title = "wv".to_owned();
    edit.replace_cover = Some(cover_png());
    write_edit(&path, &edit).unwrap();
    let tagged = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    // The audio is a stream of blocks: the file's own blocks repeated until the song is as long as a real one.
    // The APEv2 footer (the last 32 bytes) says how long the tag is; with its header (a flag) that is size + 32.
    let footer = &tagged[tagged.len() - 32..];
    assert_eq!(&footer[..8], b"APETAGEX");
    let size = u32::from_le_bytes(footer[12..16].try_into().unwrap()) as usize;
    let flags = u32::from_le_bytes(footer[20..24].try_into().unwrap());
    let tag_len = size + if flags & 0x8000_0000 != 0 { 32 } else { 0 };
    let audio = &tagged[..tagged.len() - tag_len];
    let mut file = Vec::new();
    while file.len() < AUDIO {
        file.extend_from_slice(audio);
    }
    file.extend_from_slice(&tagged[tagged.len() - tag_len..]);
    file
}

/// A real PNG of about `COVER` bytes (the tag writer of the WavPack test validates its picture).
fn cover_png() -> Vec<u8> {
    let mut state = 0x1234_5678_u32;
    let side = 320;
    let image = image::RgbImage::from_fn(side, side, |_, _| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        image::Rgb([(state >> 8) as u8, (state >> 16) as u8, (state >> 24) as u8])
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

/// A song of 28 MB with a 300 KB cover costs the tag, never the song: the tag readers touch the head of the file (the tag) and
/// a few bytes of its end (the tag probes), whatever the container.
#[test]
fn the_cover_of_a_28_mb_song_costs_the_size_of_the_tag() {
    for (bytes, name) in [
        (mp3(true), "a.mp3"),
        (flac(), "a.flac"),
        (m4a(false), "moov-last.m4a"),
        (m4a(true), "moov-first.m4a"),
        (dsf(), "a.dsf"),
    ] {
        let (cover, read, _) = read_cover(bytes, name);
        assert_eq!(cover, Some(COVER), "{name}: the cover was found");
        assert!(
            read <= COVER as u64 + 64 * 1024,
            "{name}: {read} bytes read for a {COVER} byte cover"
        );
    }
    let (cover, read, highest) = read_cover(wavpack_with_tail_tag(), "a.wv");
    let cover = cover.expect("a.wv: the APE cover was found");
    assert!(
        read <= cover as u64 + 64 * 1024,
        "a.wv: {read} bytes read for a {cover} byte cover"
    );
    assert!(highest > AUDIO as u64, "the tag is at the end");
}

/// A song without a cover costs no more than its tag probes.
#[test]
fn a_song_without_a_cover_reads_almost_nothing() {
    let mut file = flac();
    // Same file, picture block turned into padding: STREAMINFO is the last block.
    file[4] |= 0x80;
    let (cover, read, _) = read_cover(file, "none.flac");
    assert_eq!(cover, None);
    assert!(read <= 64 * 1024, "{read} bytes read");
}
