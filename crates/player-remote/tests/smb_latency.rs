//! Where does the fixed per-call delay of an idle SMB connection come from? Read-only: `stat` of one existing entry
//! with different idle gaps, one or two threads, and tiny reads. Environment as in `real_smb.rs`
//! (`MOE_SMB_HOST/PORT/SHARE/DIR/FILE`). Nothing secret is printed.
//! Run: `cargo test -p player-remote --test smb_latency --release -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use player_remote::fs::RemoteFs;
use player_remote::smb::{SmbFs, SmbSettings};
use zeroize::Zeroizing;

fn settings() -> Option<SmbSettings> {
    let host = std::env::var("MOE_SMB_HOST").ok()?;
    let share = std::env::var("MOE_SMB_SHARE").ok()?;
    let file = std::fs::read_to_string(std::env::var("MOE_SMB_FILE").ok()?).ok()?;
    let mut lines = file.lines();
    Some(SmbSettings {
        host,
        port: std::env::var("MOE_SMB_PORT").ok().and_then(|p| p.parse().ok()),
        share,
        base_path: std::env::var("MOE_SMB_DIR").unwrap_or_default(),
        user: lines.next()?.trim().to_owned(),
        password: Zeroizing::new(lines.next()?.trim().to_owned()),
    })
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

#[test]
#[ignore]
fn idle_gap_latency() {
    let Some(settings) = settings() else { return };
    let fs = SmbFs::connect(settings).expect("connect");
    let entries = fs.list("").expect("list");
    let target = entries.iter().find(|e| !e.is_dir).map(|e| e.name.clone());
    let target = target.unwrap_or_else(|| entries[0].name.clone());
    let probe = |label: &str, gap: u64, n: usize| {
        let mut times = Vec::new();
        for _ in 0..n {
            std::thread::sleep(Duration::from_millis(gap));
            let t = Instant::now();
            fs.stat(&target).expect("stat");
            times.push(ms(t.elapsed()));
        }
        println!("{label} gap {gap:>4} ms: median {:.1} ms  min {:.1}  max {:.1}", median(times.clone()), times.iter().cloned().fold(f64::MAX, f64::min), times.iter().cloned().fold(0.0, f64::max));
    };
    probe("stat", 0, 10); // warm
    for gap in [0, 10, 50, 100, 200, 500, 1000] {
        probe("stat", gap, 12);
    }
    // two threads at once
    let times = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| {
                for _ in 0..20 {
                    let t = Instant::now();
                    fs.stat(&target).unwrap();
                    times.lock().unwrap().push(ms(t.elapsed()));
                }
            });
        }
    });
    println!("stat 2 threads: median {:.1} ms", median(times.into_inner().unwrap()));
    // open + tiny read
    let mut times = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        let mut f = fs.open(&target).expect("open");
        let mut buf = [0u8; 64];
        let _ = f.read_at(0, &mut buf);
        times.push(ms(t.elapsed()));
    }
    println!("open+read64: median {:.1} ms", median(times));
}

#[test]
#[ignore]
fn raw_tcp_round_trips() {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    let host = std::env::var("MOE_SMB_HOST").unwrap();
    let port: u16 = std::env::var("MOE_SMB_PORT").unwrap().parse().unwrap();
    let addr = (host.as_str(), port).to_socket_addrs().unwrap().next().unwrap();
    let mut connects = Vec::new();
    for _ in 0..8 {
        let t = Instant::now();
        let s = TcpStream::connect(addr).unwrap();
        connects.push(ms(t.elapsed()));
        drop(s);
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("tcp connect (1 RTT): median {:.1} ms", median(connects));
    // Minimal SMB2 NEGOTIATE (dialect 2.0.2 only), sent as header+body in two writes vs one write.
    let mut body = vec![0u8; 64];
    body[..4].copy_from_slice(&[0xFE, b'S', b'M', b'B']);
    body[4] = 64; // structure size
    body[12] = 0; // command NEGOTIATE=0
    body[14] = 1; // credit request
    let mut neg = vec![0u8; 36 + 2];
    neg[0] = 36; // StructureSize
    neg[2] = 1; // DialectCount
    neg[4] = 1; // security mode
    neg[36] = 0x02; neg[37] = 0x02; // 2.0.2
    body.extend_from_slice(&neg);
    let mut framed = vec![0u8, 0, 0, body.len() as u8];
    framed.extend_from_slice(&body);
    for (label, split, nodelay) in [("one write", false, false), ("two writes", true, false), ("two writes+NODELAY", true, true)] {
        let mut times = Vec::new();
        for _ in 0..8 {
            let mut s = TcpStream::connect(addr).unwrap();
            s.set_nodelay(nodelay).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let t = Instant::now();
            if split {
                s.write_all(&framed[..4]).unwrap();
                s.write_all(&framed[4..]).unwrap();
            } else {
                s.write_all(&framed).unwrap();
            }
            let mut buf = [0u8; 4];
            s.read_exact(&mut buf).unwrap();
            times.push(ms(t.elapsed()));
            std::thread::sleep(Duration::from_millis(200));
        }
        println!("negotiate {label}: median {:.1} ms  min {:.1}", median(times.clone()), times.iter().cloned().fold(f64::MAX, f64::min));
    }
}
