//! Real songs over WebDAV (run by hand): a folder of the owner's music is served by the test WebDAV server, walked with
//! the client, and the tags of a sample of the files are read through the cached range reader; they must equal what lofty
//! reads from the local file (title, artist, album, duration, sample rate, and the cover bytes).
//!
//! `MOE_REMOTE_MUSIC=D:/Music/Loss cargo test -p player-remote --features testing --release --test real_music -- --ignored --nocapture`

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Instant;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::Accessor;
use player_remote::account::{RemoteAccount, RemoteProtocol, TlsMode};
use player_remote::fs::{RangeReader, RemoteFs};
use player_remote::test_server::{Auth, TestWebDav};
use player_remote::webdav::WebDavFs;
use zeroize::Zeroizing;

const AUDIO: [&str; 8] = ["flac", "mp3", "m4a", "wav", "ogg", "opus", "wv", "dsf"];

fn walk(fs: &WebDavFs, dir: &str, out: &mut Vec<(String, u64)>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    for entry in fs.list(dir).unwrap() {
        let path = if dir.is_empty() {
            entry.name.clone()
        } else {
            format!("{dir}/{}", entry.name)
        };
        if entry.is_dir {
            walk(fs, &path, out, limit);
        } else if Path::new(&entry.name)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| AUDIO.contains(&e.to_ascii_lowercase().as_str()))
        {
            out.push((path, entry.size));
        }
    }
}

#[test]
#[ignore = "needs MOE_REMOTE_MUSIC"]
fn tags_read_over_webdav_equal_the_local_tags() {
    let root = PathBuf::from(std::env::var("MOE_REMOTE_MUSIC").expect("MOE_REMOTE_MUSIC"));
    let server = TestWebDav::serve(
        &root,
        Auth::Digest {
            user: "me".into(),
            password: "pw".into(),
        },
    );
    let account = RemoteAccount {
        id: "t".into(),
        name: "t".into(),
        protocol: RemoteProtocol::Webdav,
        host: "127.0.0.1".into(),
        port: Some(server.address.port()),
        path: String::new(),
        username: "me".into(),
        password: None,
        tls: TlsMode::Off,
        only_on_wifi: false,
    };
    let fs = WebDavFs::new(&account, Some(Zeroizing::new("pw".to_owned()))).unwrap();
    let started = Instant::now();
    let mut files = Vec::new();
    walk(&fs, "", &mut files, 400);
    println!(
        "walked {} audio files in {:.1}s",
        files.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(!files.is_empty());
    let step = (files.len() / 40).max(1);
    let (mut checked, mut bytes_fetched) = (0, 0u64);
    for (path, size) in files.iter().step_by(step) {
        let local = lofty::read_from_path(root.join(path)).unwrap();
        let mut reader = RangeReader::new(fs.open(path).unwrap());
        assert_eq!(reader.len(), *size, "{path}");
        let probe = lofty::probe::Probe::new(BufReader::new(&mut reader))
            .guess_file_type()
            .unwrap();
        let remote = probe
            .read()
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        let (lt, rt) = (local.primary_tag(), remote.primary_tag());
        assert_eq!(
            lt.and_then(|t| t.title().map(|v| v.into_owned())),
            rt.and_then(|t| t.title().map(|v| v.into_owned())),
            "{path}: title"
        );
        assert_eq!(
            lt.and_then(|t| t.artist().map(|v| v.into_owned())),
            rt.and_then(|t| t.artist().map(|v| v.into_owned())),
            "{path}: artist"
        );
        assert_eq!(
            lt.and_then(|t| t.album().map(|v| v.into_owned())),
            rt.and_then(|t| t.album().map(|v| v.into_owned())),
            "{path}: album"
        );
        assert_eq!(
            local.properties().duration(),
            remote.properties().duration(),
            "{path}: duration"
        );
        assert_eq!(
            local.properties().sample_rate(),
            remote.properties().sample_rate(),
            "{path}: sample rate"
        );
        let cover = |tag: Option<&lofty::tag::Tag>| {
            tag.and_then(|t| t.pictures().first().map(|p| p.data().to_vec()))
        };
        assert_eq!(cover(lt), cover(rt), "{path}: cover bytes");
        bytes_fetched += reader.requests();
        checked += 1;
    }
    println!(
        "{checked} files: tags, durations and covers over WebDAV equal the local reads; {} requests, {} GETs in total, {:.1}s",
        bytes_fetched,
        server.count("GET"),
        started.elapsed().as_secs_f64()
    );
}
