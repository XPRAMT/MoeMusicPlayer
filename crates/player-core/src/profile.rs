//! Opt-in in-process profiling of library builds (ADR 0027).
//!
//! Off by default. When `MOE_LIBRARY_PROFILE=<file.jsonl>` is set (or [`init`] is called), phase events are appended to that
//! file as JSON Lines: every line carries a UTC RFC 3339 timestamp, the same instant in the local offset (taken from
//! `MOE_LIBRARY_PROFILE_UTC_OFFSET_MINUTES`, else UTC), a monotonic `mono_ms` since the profiler started, the pid and the
//! thread. When disabled every hook is a single relaxed atomic load.

use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Mutex, OnceLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

struct Profiler {
    started: Instant,
    offset_minutes: i32,
    path: PathBuf,
    out: Mutex<BufWriter<File>>,
}

static PROFILER: OnceLock<Option<Profiler>> = OnceLock::new();
/// 0 = not decided, 1 = off, 2 = on.
static STATE: AtomicU8 = AtomicU8::new(0);

fn open(path: &Path) -> Option<Profiler> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = OpenOptions::new().create(true).append(true).open(path).ok()?;
    let offset_minutes = std::env::var("MOE_LIBRARY_PROFILE_UTC_OFFSET_MINUTES")
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok())
        .filter(|value| value.abs() <= 14 * 60)
        .unwrap_or(0);
    Some(Profiler {
        started: Instant::now(),
        offset_minutes,
        path: path.to_path_buf(),
        out: Mutex::new(BufWriter::new(file)),
    })
}

fn profiler() -> Option<&'static Profiler> {
    if STATE.load(Ordering::Relaxed) == 1 {
        return None;
    }
    let profiler = PROFILER
        .get_or_init(|| {
            std::env::var_os("MOE_LIBRARY_PROFILE")
                .filter(|value| !value.is_empty())
                .and_then(|value| open(Path::new(&value)))
        })
        .as_ref();
    STATE.store(if profiler.is_some() { 2 } else { 1 }, Ordering::Relaxed);
    profiler
}

/// Turn profiling on with an explicit output file (a benchmark binary). Returns false when the profiler was already
/// decided (for example by the environment variable) or the file cannot be opened.
pub fn init(path: impl AsRef<Path>) -> bool {
    let mut opened = false;
    PROFILER.get_or_init(|| {
        let profiler = open(path.as_ref());
        opened = profiler.is_some();
        profiler
    });
    STATE.store(
        if PROFILER.get().is_some_and(Option::is_some) { 2 } else { 1 },
        Ordering::Relaxed,
    );
    opened
}

pub fn enabled() -> bool {
    profiler().is_some()
}

/// The profile file in use, if any.
pub fn output_path() -> Option<PathBuf> {
    profiler().map(|p| p.path.clone())
}

/// Append one event. `fields` is a list of already-encoded JSON members (use [`field`], [`str_field`]).
pub fn event(name: &str, fields: &[String]) {
    let Some(profiler) = profiler() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mono_ms = profiler.started.elapsed().as_secs_f64() * 1000.0;
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("").to_owned();
    let mut line = format!(
        "{{\"ts\":\"{}\",\"ts_local\":\"{}\",\"utc_ms\":{now},\"mono_ms\":{mono_ms:.3},\"pid\":{},\"thread\":{},\"event\":{}",
        rfc3339(now, 0),
        rfc3339(now, profiler.offset_minutes),
        std::process::id(),
        json_string(&format!("{:?}{}", thread.id(), if thread_name.is_empty() { String::new() } else { format!(" {thread_name}") })),
        json_string(name),
    );
    for member in fields {
        line.push(',');
        line.push_str(member);
    }
    line.push_str("}\n");
    if let Ok(mut out) = profiler.out.lock() {
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    }
}

/// Emit `library_ready` and write `<profile file>.ready.json` holding the same members, so an external monitor can
/// wait for one file instead of guessing completion from resource counters.
pub fn mark_ready(fields: &[String]) {
    let Some(profiler) = profiler() else {
        return;
    };
    event("library_ready", fields);
    let mut marker = profiler.path.as_os_str().to_os_string();
    marker.push(".ready.json");
    let _ = std::fs::write(PathBuf::from(marker), format!("{{{}}}\n", fields.join(",")));
}

/// A numeric or raw-JSON member.
pub fn field(name: &str, value: impl std::fmt::Display) -> String {
    format!("{}:{value}", json_string(name))
}

pub fn str_field(name: &str, value: &str) -> String {
    format!("{}:{}", json_string(name), json_string(value))
}

pub fn ms(duration: std::time::Duration) -> String {
    format!("{:.3}", duration.as_secs_f64() * 1000.0)
}

pub fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Percentile summary and slowest items of a set of per-item durations, as JSON members.
pub fn distribution_fields(prefix: &str, durations_us: &mut [u64]) -> Vec<String> {
    durations_us.sort_unstable();
    let pick = |q: f64| -> f64 {
        if durations_us.is_empty() {
            return 0.0;
        }
        let index = ((durations_us.len() - 1) as f64 * q).round() as usize;
        durations_us[index] as f64 / 1000.0
    };
    let sum: u64 = durations_us.iter().sum();
    vec![
        field(&format!("{prefix}_count"), durations_us.len()),
        field(&format!("{prefix}_sum_ms"), format!("{:.3}", sum as f64 / 1000.0)),
        field(&format!("{prefix}_p50_ms"), format!("{:.3}", pick(0.5))),
        field(&format!("{prefix}_p90_ms"), format!("{:.3}", pick(0.9))),
        field(&format!("{prefix}_p99_ms"), format!("{:.3}", pick(0.99))),
        field(&format!("{prefix}_max_ms"), format!("{:.3}", pick(1.0))),
    ]
}

fn rfc3339(utc_ms: i64, offset_minutes: i32) -> String {
    let local_ms = utc_ms + offset_minutes as i64 * 60_000;
    let days = local_ms.div_euclid(86_400_000);
    let day_ms = local_ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let (h, m, s, milli) = (
        day_ms / 3_600_000,
        day_ms / 60_000 % 60,
        day_ms / 1000 % 60,
        day_ms % 1000,
    );
    let zone = if offset_minutes == 0 {
        "Z".to_owned()
    } else {
        let sign = if offset_minutes < 0 { '-' } else { '+' };
        let abs = offset_minutes.abs();
        format!("{sign}{:02}:{:02}", abs / 60, abs % 60)
    };
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{milli:03}{zone}")
}

/// Howard Hinnant's days-from-civil inverse.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rfc3339_with_offset() {
        // 2026-10-21T17:50:00Z
        let utc = 1_792_605_000_000;
        assert_eq!(rfc3339(utc, 0), "2026-10-21T17:50:00.000Z");
        assert_eq!(rfc3339(utc, 480), "2026-10-22T01:50:00.000+08:00");
        assert_eq!(rfc3339(utc, -90), "2026-10-21T16:20:00.000-01:30");
    }

    #[test]
    fn escapes_json() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn distribution_percentiles() {
        let mut values = vec![5_000, 1_000, 3_000, 2_000, 4_000];
        let fields = distribution_fields("x", &mut values);
        assert!(fields.contains(&"\"x_p50_ms\":3.000".to_owned()));
        assert!(fields.contains(&"\"x_max_ms\":5.000".to_owned()));
    }
}
