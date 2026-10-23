//! Measurement helper (ADR 0034): DWM composition statistics per monitor target and the CPU use of one process.
//! `cargo run --release -p player-lyrics-native --example measure -- <pid> [seconds]`

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::Graphics::DirectComposition::*;
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let pid: u32 = args.first().and_then(|s| s.parse().ok()).expect("pid");
    let seconds: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10.0);

    fn targets() -> Vec<(COMPOSITION_TARGET_ID, COMPOSITION_TARGET_STATS)> {
        unsafe {
            let Ok(frame) = DCompositionGetFrameId(COMPOSITION_FRAME_ID_COMPLETED) else {
                return Vec::new();
            };
            let mut fs = COMPOSITION_FRAME_STATS::default();
            let mut ids = [COMPOSITION_TARGET_ID::default(); 16];
            let mut count = 0u32;
            if DCompositionGetStatistics(
                frame,
                &mut fs,
                ids.len() as u32,
                Some(ids.as_mut_ptr()),
                Some(&mut count),
            )
            .is_err()
            {
                return Vec::new();
            }
            ids[..count.min(16) as usize]
                .iter()
                .filter_map(|id| {
                    DCompositionGetTargetStatistics(frame, id)
                        .ok()
                        .map(|s| (*id, s))
                })
                .collect()
        }
    }
    let cpu = |pid: u32| -> f64 {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return 0.0;
            };
            let (mut c, mut e, mut k, mut u) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let _ = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
            let _ = CloseHandle(h);
            let f = |t: FILETIME| {
                (((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) as f64 / 1e7
            };
            f(k) + f(u)
        }
    };
    let start = std::time::Instant::now();
    let (c0, t0) = (cpu(pid), targets());
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    let elapsed = start.elapsed().as_secs_f64();
    let (c1, t1) = (cpu(pid), targets());
    println!(
        "pid {pid}: CPU {:.2}% of one core over {:.1}s",
        (c1 - c0) / elapsed * 100.0,
        elapsed
    );
    for (id, end) in &t1 {
        if let Some((_, begin)) = t0.iter().find(|(b, _)| b == id) {
            let (p, q) = (end.presentedStats, begin.presentedStats);
            println!(
                "target vidpn={} uid={}: presents {:.1}/s refreshes {:.1}/s (vblank {:.2} ms)",
                id.vidPnSourceId,
                id.uniqueId,
                p.presentCount.wrapping_sub(q.presentCount) as f64 / elapsed,
                p.refreshCount.wrapping_sub(q.refreshCount) as f64 / elapsed,
                end.vblankDuration as f64 / 1e4
            );
        }
    }
}
