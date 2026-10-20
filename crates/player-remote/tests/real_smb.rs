//! Runs the SMB client against a real server (ADR 0026). Skipped unless the environment says where it is:
//!
//! - `MOE_SMB_HOST`, `MOE_SMB_PORT` (default 445), `MOE_SMB_SHARE`
//! - `MOE_SMB_FILE`: a text file, the user name on the first line and the password on the second
//! - `MOE_SMB_DIR`: a folder inside the share to list and read from (default: the share's root)
//!
//! Nothing secret is printed.

use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

use player_remote::fs::{RangeReader, RemoteError, RemoteFs};
use player_remote::smb::{SmbFs, SmbSettings};
use zeroize::Zeroizing;

fn settings() -> Option<SmbSettings> {
    let host = std::env::var("MOE_SMB_HOST").ok()?;
    let share = std::env::var("MOE_SMB_SHARE").ok()?;
    let file = std::fs::read_to_string(std::env::var("MOE_SMB_FILE").ok()?).ok()?;
    let mut lines = file.lines();
    Some(SmbSettings {
        host,
        port: std::env::var("MOE_SMB_PORT")
            .ok()
            .and_then(|p| p.parse().ok()),
        share,
        base_path: std::env::var("MOE_SMB_DIR").unwrap_or_default(),
        user: lines.next()?.trim().to_owned(),
        password: Zeroizing::new(lines.next()?.trim().to_owned()),
    })
}

fn is_audio(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        "flac", "mp3", "wav", "wv", "dsf", "opus", "ogg", "m4a", "ape",
    ]
    .iter()
    .any(|extension| lower.ends_with(&format!(".{extension}")))
}

#[test]
fn lists_the_root_and_stats_entries() {
    let Some(settings) = settings() else {
        eprintln!("skipped: MOE_SMB_* is not set");
        return;
    };
    let fs = SmbFs::connect(settings).expect("connect");
    let entries = fs.list("").expect("list the root");
    println!("{} entries at the root", entries.len());
    for entry in entries.iter().take(40) {
        println!(
            "{} {:>12} {}",
            if entry.is_dir { "d" } else { "-" },
            entry.size,
            entry.name
        );
    }
    let first = entries.first().expect("a non-empty root");
    let stat = fs.stat(&first.name).expect("stat the first entry");
    assert_eq!(stat.is_dir, first.is_dir);
    assert_eq!(stat.size, first.size);
    assert_eq!(stat.modified_ms, first.modified_ms);
    assert!(matches!(
        fs.stat("definitely-not-here-1234").unwrap_err(),
        RemoteError::NotFound
    ));
    assert!(matches!(
        fs.list("../x").unwrap_err(),
        RemoteError::NotFound
    ));
}

#[test]
fn a_wrong_password_is_an_auth_error() {
    let Some(mut settings) = settings() else {
        eprintln!("skipped: MOE_SMB_* is not set");
        return;
    };
    settings.password = Zeroizing::new("this-is-not-the-password".to_owned());
    match SmbFs::connect(settings) {
        Err(error) => assert_eq!(error, RemoteError::Auth, "{error}"),
        Ok(_) => panic!("a wrong password was accepted"),
    }
}

/// Reads the first audio file found (searching a few levels down) through the cached range reader, with seeks, and
/// reports the throughput.
#[test]
fn reads_an_audio_file_with_seeks_and_measures_speed() {
    let Some(settings) = settings() else {
        eprintln!("skipped: MOE_SMB_* is not set");
        return;
    };
    let fs = SmbFs::connect(settings).expect("connect");
    let mut found = None;
    let mut queue = vec![String::new()];
    while let Some(dir) = queue.pop() {
        if found.is_some() || queue.len() > 200 {
            break;
        }
        let Ok(entries) = fs.list(&dir) else { continue };
        for entry in entries {
            let path = if dir.is_empty() {
                entry.name.clone()
            } else {
                format!("{dir}/{}", entry.name)
            };
            if entry.is_dir {
                queue.insert(0, path);
            } else if is_audio(&entry.name) && entry.size > 1_000_000 && found.is_none() {
                found = Some((path, entry.size));
            }
        }
    }
    let (path, size) = found.expect("an audio file under the folder");
    println!("reading a {size} byte file");

    let file = fs.open(&path).expect("open");
    assert_eq!(file.len(), size);
    let read_ahead = std::env::var("MOE_SMB_READ_AHEAD")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(player_remote::smb::SMB_READ_AHEAD_BLOCKS);
    let mut reader = RangeReader::new(file).with_read_ahead(read_ahead);

    let started = Instant::now();
    let mut head = vec![0u8; 4];
    reader.read_exact(&mut head).expect("read the head");
    println!("head {head:?} after {:?}", started.elapsed());

    // The whole file, sequentially.
    reader.seek(SeekFrom::Start(0)).unwrap();
    let started = Instant::now();
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    let limit = 64 * 1024 * 1024u64;
    while total < size.min(limit) {
        let read = reader.read(&mut buffer).expect("read");
        if read == 0 {
            break;
        }
        total += read as u64;
    }
    let seconds = started.elapsed().as_secs_f64().max(0.001);
    println!(
        "read {} MiB in {:.2}s = {:.1} MiB/s ({} requests)",
        total / (1 << 20),
        seconds,
        total as f64 / (1 << 20) as f64 / seconds,
        reader.requests()
    );
    assert!(total > 0);

    // A read at the end equals the same bytes read through a fresh reader (seek correctness).
    let tail_start = size.saturating_sub(4096);
    reader.seek(SeekFrom::Start(tail_start)).unwrap();
    let mut tail = Vec::new();
    reader.read_to_end(&mut tail).unwrap();
    let mut again = RangeReader::new(fs.open(&path).unwrap());
    again.seek(SeekFrom::Start(tail_start)).unwrap();
    let mut tail_again = Vec::new();
    again.read_to_end(&mut tail_again).unwrap();
    assert_eq!(tail, tail_again);
    assert_eq!(tail.len() as u64, size - tail_start);
}

/// Four threads read different files through one connection at the same time (what the library scan does).
#[test]
fn four_threads_read_through_one_connection() {
    let Some(settings) = settings() else {
        eprintln!("skipped: MOE_SMB_* is not set");
        return;
    };
    let fs = std::sync::Arc::new(SmbFs::connect(settings).expect("connect"));
    let mut files = Vec::new();
    let mut queue = vec![String::new()];
    while let Some(dir) = queue.pop() {
        if files.len() >= 8 {
            break;
        }
        let Ok(entries) = fs.list(&dir) else { continue };
        for entry in entries {
            let path = if dir.is_empty() {
                entry.name.clone()
            } else {
                format!("{dir}/{}", entry.name)
            };
            if entry.is_dir {
                queue.push(path);
            } else if is_audio(&entry.name) && entry.size > 200_000 && files.len() < 8 {
                files.push((path, entry.size));
            }
        }
    }
    assert!(files.len() >= 4, "not enough audio files under the folder");
    let started = Instant::now();
    let handles: Vec<_> = files
        .into_iter()
        .map(|(path, size)| {
            let fs = std::sync::Arc::clone(&fs);
            std::thread::spawn(move || {
                let mut reader = fs.open_reader(&path).expect("open");
                let mut head = vec![0u8; 262_144.min(size as usize)];
                reader.read_exact(&mut head).expect("read the head");
                reader.seek(SeekFrom::End(-1024.min(size as i64))).unwrap();
                let mut tail = Vec::new();
                reader.read_to_end(&mut tail).unwrap();
                (head.len(), tail.len())
            })
        })
        .collect();
    for handle in handles {
        let (head, tail) = handle.join().expect("a reader thread panicked");
        assert!(head > 0 && tail > 0);
    }
    println!(
        "8 files, head and tail each, 4+ threads: {:?}",
        started.elapsed()
    );
}
