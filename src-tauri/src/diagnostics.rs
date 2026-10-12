use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    process::Command,
    sync::Once,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_PANIC_LOG_BYTES: u64 = 1024 * 1024;
const MAX_REPORT_LOG_BYTES: u64 = 512 * 1024;
const MAX_PANIC_RECORD_BYTES: usize = 32 * 1024;
const MAX_RECENT_PANICS: usize = 12;

static INSTALL_PANIC_HOOK: Once = Once::new();

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PanicRecord {
    occurred_at_utc: String,
    process_id: u32,
    thread_id: String,
    thread_name: Option<String>,
    message: String,
    source_file: Option<String>,
    source_line: Option<u32>,
    source_column: Option<u32>,
    backtrace: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeSnapshot {
    captured_at_utc: String,
    process_id: u32,
    database_initialized: bool,
    database_error: Option<String>,
    audio_service_initialized: bool,
    audio_service_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileSummary {
    exists: bool,
    bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticReport {
    schema_version: u32,
    captured_at_utc: String,
    application: ApplicationIdentity,
    diagnostic_process: DiagnosticProcess,
    user_data: UserDataSummary,
    other_process_ids_with_same_image_name: Vec<u32>,
    runtime_snapshot: Option<RuntimeSnapshot>,
    recent_panics: Vec<PanicRecord>,
    privacy: PrivacySummary,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationIdentity {
    name: &'static str,
    version: &'static str,
    build_timestamp_utc: &'static str,
    git_commit: &'static str,
    architecture: &'static str,
    operating_system: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticProcess {
    process_id: u32,
    executable_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserDataSummary {
    path: String,
    database: FileSummary,
    database_wal: FileSummary,
    database_shm: FileSummary,
    settings: FileSummary,
    panic_log_path: String,
    runtime_snapshot_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrivacySummary {
    includes_media_metadata: bool,
    includes_settings_contents: bool,
    includes_database_contents: bool,
    includes_executable_and_user_data_paths: bool,
    includes_panic_backtraces: bool,
}

pub(crate) fn install_panic_hook(user_data_dir: PathBuf) {
    INSTALL_PANIC_HOOK.call_once(move || {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let record = panic_record(info);
                append_panic_record(&user_data_dir, &record);
            }));
            previous(info);
        }));
    });
}

pub(crate) fn write_runtime_snapshot(
    user_data_dir: &Path,
    database_initialized: bool,
    database_error: Option<&str>,
    audio_service_initialized: bool,
    audio_service_error: Option<&str>,
) -> std::io::Result<()> {
    let diagnostics_dir = user_data_dir.join("diagnostics");
    fs::create_dir_all(&diagnostics_dir)?;
    let destination = diagnostics_dir.join("runtime.json");
    let temporary = diagnostics_dir.join("runtime.json.tmp");
    let snapshot = RuntimeSnapshot {
        captured_at_utc: utc_timestamp(SystemTime::now()),
        process_id: std::process::id(),
        database_initialized,
        database_error: database_error.map(|error| truncate_and_redact_paths(error, 4096)),
        audio_service_initialized,
        audio_service_error: audio_service_error
            .map(|error| truncate_and_redact_paths(error, 4096)),
    };
    let mut bytes = serde_json::to_vec(&snapshot).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    replace_file(&temporary, &destination)?;
    Ok(())
}

pub(crate) fn report_json() -> Result<String, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let user_data = crate::windows_data_directory::user_data_dir_for_executable(&executable)
        .map_err(|error| error.to_string())?;
    let report = collect_report(&executable, &user_data, other_process_ids(&executable));
    serde_json::to_string_pretty(&report).map_err(|error| error.to_string())
}

fn collect_report(
    executable: &Path,
    user_data: &Path,
    other_process_ids_with_same_image_name: Vec<u32>,
) -> DiagnosticReport {
    let database_path = user_data.join("moemusicplayer.sqlite3");
    let diagnostics_dir = user_data.join("diagnostics");
    let panic_log_path = diagnostics_dir.join("panic.jsonl");
    let runtime_snapshot_path = diagnostics_dir.join("runtime.json");
    DiagnosticReport {
        schema_version: 1,
        captured_at_utc: utc_timestamp(SystemTime::now()),
        application: ApplicationIdentity {
            name: "MoeMusicPlayer",
            version: env!("MOE_BUILD_VERSION"),
            build_timestamp_utc: env!("MOE_BUILD_TIMESTAMP_UTC"),
            git_commit: env!("MOE_GIT_COMMIT"),
            architecture: std::env::consts::ARCH,
            operating_system: std::env::consts::OS,
        },
        diagnostic_process: DiagnosticProcess {
            process_id: std::process::id(),
            executable_path: executable.display().to_string(),
        },
        user_data: UserDataSummary {
            path: user_data.display().to_string(),
            database: file_summary(&database_path),
            database_wal: file_summary(&sidecar_path(&database_path, "-wal")),
            database_shm: file_summary(&sidecar_path(&database_path, "-shm")),
            settings: file_summary(&user_data.join("settings.json")),
            panic_log_path: panic_log_path.display().to_string(),
            runtime_snapshot_path: runtime_snapshot_path.display().to_string(),
        },
        other_process_ids_with_same_image_name,
        runtime_snapshot: read_runtime_snapshot(&runtime_snapshot_path),
        recent_panics: read_recent_panics(&panic_log_path),
        privacy: PrivacySummary {
            includes_media_metadata: false,
            includes_settings_contents: false,
            includes_database_contents: false,
            includes_executable_and_user_data_paths: true,
            includes_panic_backtraces: true,
        },
    }
}

fn panic_record(info: &PanicHookInfo<'_>) -> PanicRecord {
    let (source_file, source_line, source_column) = info
        .location()
        .map(|location| {
            (
                location
                    .file()
                    .rsplit(['\\', '/'])
                    .next()
                    .map(str::to_owned),
                Some(location.line()),
                Some(location.column()),
            )
        })
        .unwrap_or((None, None, None));
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned());
    PanicRecord {
        occurred_at_utc: utc_timestamp(SystemTime::now()),
        process_id: std::process::id(),
        thread_id: format!("{:?}", std::thread::current().id()),
        thread_name: std::thread::current().name().map(str::to_owned),
        message: truncate_and_redact_paths(&message, 4096),
        source_file,
        source_line,
        source_column,
        backtrace: truncate_and_redact_paths(
            &std::backtrace::Backtrace::force_capture().to_string(),
            MAX_PANIC_RECORD_BYTES / 2,
        ),
    }
}

fn append_panic_record(user_data: &Path, record: &PanicRecord) {
    let diagnostics_dir = user_data.join("diagnostics");
    if fs::create_dir_all(&diagnostics_dir).is_err() {
        return;
    }
    let log_path = diagnostics_dir.join("panic.jsonl");
    if fs::metadata(&log_path)
        .map(|metadata| metadata.len() >= MAX_PANIC_LOG_BYTES)
        .unwrap_or(false)
    {
        let backup = diagnostics_dir.join("panic.previous.jsonl");
        let _ = fs::remove_file(&backup);
        let _ = fs::rename(&log_path, backup);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) else {
        return;
    };
    let Ok(mut bytes) = serde_json::to_vec(record) else {
        return;
    };
    bytes.push(b'\n');
    let _ = file.write_all(&bytes);
    let _ = file.flush();
}

fn read_recent_panics(path: &Path) -> Vec<PanicRecord> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(metadata) = file.metadata() else {
        return Vec::new();
    };
    let start = metadata.len().saturating_sub(MAX_REPORT_LOG_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file
        .take(MAX_REPORT_LOG_BYTES)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Vec::new();
    }
    let content = String::from_utf8_lossy(&bytes);
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<PanicRecord>(line).ok())
        .rev()
        .take(MAX_RECENT_PANICS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn read_runtime_snapshot(path: &Path) -> Option<RuntimeSnapshot> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn file_summary(path: &Path) -> FileSummary {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => FileSummary {
            exists: true,
            bytes: Some(metadata.len()),
        },
        _ => FileSummary {
            exists: false,
            bytes: None,
        },
    }
}

fn sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut value = database.as_os_str().to_os_string();
    value.push(suffix);
    value.into()
}

fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}

fn other_process_ids(executable: &Path) -> Vec<u32> {
    let Some(name) = executable.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {name}"), "/FO", "CSV", "/NH"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(',');
            let image = fields.next()?.trim_matches('"');
            if !image.eq_ignore_ascii_case(name) {
                return None;
            }
            let process_id = fields.next()?.trim_matches('"').parse().ok()?;
            (process_id != std::process::id()).then_some(process_id)
        })
        .collect()
}

fn truncate_and_redact_paths(value: &str, max_bytes: usize) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len().min(max_bytes));
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        let is_drive_path = character.is_ascii_alphabetic()
            && characters.get(index + 1) == Some(&':')
            && characters
                .get(index + 2)
                .is_some_and(|byte| *byte == '\\' || *byte == '/');
        let is_unc_path = (character == '\\' && characters.get(index + 1) == Some(&'\\'))
            || (character == '/' && characters.get(index + 1) == Some(&'/'));
        if is_drive_path || is_unc_path {
            push_limited(&mut output, "[path]", max_bytes);
            index += if is_drive_path { 3 } else { 2 };
            while index < characters.len() {
                let next = characters[index];
                if matches!(next, '\n' | '\r' | '"' | '\'' | ')' | ']' | '}' | ';' | ',')
                    || (next == ':'
                        && characters
                            .get(index + 1)
                            .is_some_and(|following| following.is_whitespace()))
                {
                    break;
                }
                index += 1;
            }
        } else {
            push_limited(&mut output, &character.to_string(), max_bytes);
            index += 1;
        }
        if output.len() >= max_bytes {
            break;
        }
    }
    output
}

fn push_limited(output: &mut String, value: &str, max_bytes: usize) {
    for character in value.chars() {
        if output.len() + character.len_utf8() > max_bytes {
            return;
        }
        output.push(character);
    }
}

fn utc_timestamp(now: SystemTime) -> String {
    let elapsed = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = elapsed.as_secs();
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        elapsed.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::{
        append_panic_record, collect_report, read_recent_panics, truncate_and_redact_paths,
        utc_timestamp, write_runtime_snapshot, PanicRecord, MAX_PANIC_LOG_BYTES,
    };
    use std::{fs, path::PathBuf, time::SystemTime};

    fn temp_dir() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "moemusicplayer-diagnostics-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn report_only_includes_file_metadata_and_explicitly_excludes_user_content() {
        let directory = temp_dir();
        let user_data = directory.join("UserData");
        fs::create_dir_all(&user_data).unwrap();
        fs::write(user_data.join("settings.json"), b"private-setting").unwrap();
        fs::write(
            user_data.join("moemusicplayer.sqlite3"),
            b"private-track-data",
        )
        .unwrap();
        write_runtime_snapshot(&user_data, true, None, false, None).unwrap();

        let report = collect_report(
            &directory.join("moemusicplayer.exe"),
            &user_data,
            vec![1234],
        );
        let json = serde_json::to_string(&report).unwrap();

        assert!(report.user_data.database.exists);
        assert!(report.user_data.settings.exists);
        assert_eq!(report.other_process_ids_with_same_image_name, vec![1234]);
        assert!(report.runtime_snapshot.unwrap().database_initialized);
        assert!(!report.privacy.includes_media_metadata);
        assert!(!report.privacy.includes_settings_contents);
        assert!(!report.privacy.includes_database_contents);
        assert!(!json.contains("private-setting"));
        assert!(!json.contains("private-track-data"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn panic_path_redaction_removes_drive_and_unc_paths() {
        let redacted = truncate_and_redact_paths(
            "failed to open D:\\Music\\private album\\song.mp3: \\\\server\\share\\x.flac",
            4096,
        );
        assert!(!redacted.contains("D:\\Music"));
        assert!(!redacted.contains("\\\\server"));
        assert!(redacted.contains("[path]"));
    }

    #[test]
    fn recent_panic_reader_is_bounded_to_valid_json_records() {
        let directory = temp_dir();
        let path = directory.join("panic.jsonl");
        let record = PanicRecord {
            occurred_at_utc: "2026-10-12T00:00:00.000Z".to_owned(),
            process_id: 7,
            thread_id: "ThreadId(1)".to_owned(),
            thread_name: None,
            message: "test panic".to_owned(),
            source_file: Some("lib.rs".to_owned()),
            source_line: Some(10),
            source_column: Some(2),
            backtrace: "frame".to_owned(),
        };
        fs::write(
            &path,
            format!(
                "{}\n{{invalid json}}\n",
                serde_json::to_string(&record).unwrap()
            ),
        )
        .unwrap();
        assert_eq!(read_recent_panics(&path).len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn panic_records_are_saved_and_log_rotation_keeps_one_bounded_backup() {
        let directory = temp_dir();
        let record = PanicRecord {
            occurred_at_utc: "2026-10-12T00:00:00.000Z".to_owned(),
            process_id: 7,
            thread_id: "ThreadId(1)".to_owned(),
            thread_name: Some("database-worker".to_owned()),
            message: truncate_and_redact_paths(
                "open failed D:\\Music\\private\\song.flac: denied",
                4096,
            ),
            source_file: Some("database.rs".to_owned()),
            source_line: Some(25),
            source_column: Some(4),
            backtrace: "frame 1".to_owned(),
        };
        append_panic_record(&directory, &record);
        let log = directory.join("diagnostics").join("panic.jsonl");
        assert_eq!(read_recent_panics(&log).len(), 1);
        assert!(!fs::read_to_string(&log).unwrap().contains("D:\\Music"));

        fs::write(&log, vec![b'x'; MAX_PANIC_LOG_BYTES as usize]).unwrap();
        append_panic_record(&directory, &record);
        let backup = directory.join("diagnostics").join("panic.previous.jsonl");
        assert_eq!(fs::metadata(backup).unwrap().len(), MAX_PANIC_LOG_BYTES);
        assert!(fs::metadata(log).unwrap().len() < MAX_PANIC_LOG_BYTES);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn diagnostic_timestamps_are_explicit_utc_rfc3339() {
        assert_eq!(
            utc_timestamp(std::time::UNIX_EPOCH),
            "1970-01-01T00:00:00.000Z"
        );
    }
}
