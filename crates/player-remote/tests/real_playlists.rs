//! Lists the playlist files (.m3u, .m3u8) of a real server and prints the first lines of the first ones (ADR 0026).
//! Same environment as `real_smb.rs`; skipped without it. Nothing secret is printed.

use player_remote::fs::RemoteFs;
use player_remote::smb::{SmbFs, SmbSettings};
use zeroize::Zeroizing;

fn walk(fs: &SmbFs, dir: &str, depth: usize, found: &mut Vec<String>) {
    if depth > 4 || found.len() >= 20 {
        return;
    }
    let Ok(entries) = fs.list(dir) else { return };
    for entry in entries {
        if entry.name.starts_with('.') {
            continue;
        }
        let path = if dir.is_empty() { entry.name.clone() } else { format!("{dir}/{}", entry.name) };
        if entry.is_dir {
            walk(fs, &path, depth + 1, found);
        } else if entry.name.to_lowercase().ends_with(".m3u") || entry.name.to_lowercase().ends_with(".m3u8") {
            found.push(path);
        }
    }
}

#[test]
fn finds_playlists() {
    let (Ok(host), Ok(share), Ok(file)) = (
        std::env::var("MOE_SMB_HOST"),
        std::env::var("MOE_SMB_SHARE"),
        std::env::var("MOE_SMB_FILE"),
    ) else {
        eprintln!("skipped");
        return;
    };
    let text = std::fs::read_to_string(file).unwrap();
    let mut lines = text.lines();
    let fs = SmbFs::connect(SmbSettings {
        host,
        port: std::env::var("MOE_SMB_PORT").ok().and_then(|p| p.parse().ok()),
        share,
        base_path: String::new(),
        user: lines.next().unwrap().trim().to_owned(),
        password: Zeroizing::new(lines.next().unwrap().trim().to_owned()),
    })
    .unwrap();
    let mut found = Vec::new();
    walk(&fs, "", 0, &mut found);
    for path in &found {
        println!("PLAYLIST {path}");
    }
    for path in found.iter().take(3) {
        let mut file = fs.open(path).unwrap();
        let mut buffer = vec![0_u8; file.len().min(600) as usize];
        let n = file.read_at(0, &mut buffer).unwrap();
        println!("---- {path} ({} bytes)\n{}", file.len(), String::from_utf8_lossy(&buffer[..n]));
    }
}
