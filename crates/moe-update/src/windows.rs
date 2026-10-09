use crate::{file_sha256, read_json, validate_pending, verify_file, ApplyPlan};
use std::{
    ffi::c_void,
    io::Write,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

type Handle = *mut c_void;
#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetProcessTimes(
        handle: Handle,
        created: *mut FileTime,
        exited: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn QueryFullProcessImageNameW(
        handle: Handle,
        flags: u32,
        name: *mut u16,
        length: *mut u32,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn ReplaceFileW(
        target: *const u16,
        replacement: *const u16,
        backup: *const u16,
        flags: u32,
        exclude: Handle,
        reserved: Handle,
    ) -> i32;
    fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
}

struct ProcessHandle(Handle);
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn system_error() -> String {
    std::io::Error::last_os_error().to_string()
}

fn process(pid: u32) -> Result<ProcessHandle, String> {
    let handle = unsafe { OpenProcess(0x0010_0000 | 0x1000, 0, pid) };
    if handle.is_null() {
        Err(system_error())
    } else {
        Ok(ProcessHandle(handle))
    }
}
fn started(handle: &ProcessHandle) -> Result<u64, String> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    if unsafe { GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0
    {
        return Err(system_error());
    }
    Ok(u64::from(created.low) | (u64::from(created.high) << 32))
}
pub fn process_started(pid: u32) -> Result<u64, String> {
    started(&process(pid)?)
}
fn process_image(handle: &ProcessHandle) -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle.0, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err(system_error());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(
        &buffer[..length as usize],
    )))
}

pub fn wait_for_parent(
    pid: u32,
    expected_started: u64,
    expected_image: &Path,
    timeout: Duration,
    ready: impl FnOnce(),
) -> Result<(), String> {
    // Hold the exact process object throughout the wait; PID reuse cannot turn
    // another process's exit into permission to replace the main executable.
    let parent = process(pid)?;
    if started(&parent)? != expected_started
        || process_image(&parent)?
            .canonicalize()
            .map_err(|error| error.to_string())?
            != expected_image
                .canonicalize()
                .map_err(|error| error.to_string())?
    {
        return Err("更新父程序身分不符。".into());
    }
    ready();
    match unsafe {
        WaitForSingleObject(
            parent.0,
            timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32,
        )
    } {
        0 => Ok(()),
        258 => Err("更新等待原程式正常退出逾時；未替換任何檔案。".into()),
        _ => Err(system_error()),
    }
}

fn replace(target: &Path, replacement: &Path) -> Result<(), String> {
    let target_wide = wide(target);
    let replacement_wide = wide(replacement);
    let result = unsafe {
        if target.exists() {
            ReplaceFileW(
                target_wide.as_ptr(),
                replacement_wide.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } else {
            MoveFileExW(replacement_wide.as_ptr(), target_wide.as_ptr(), 1 | 8)
        }
    };
    if result == 0 {
        Err(system_error())
    } else {
        Ok(())
    }
}

fn restore(target: &Path, backup: &Path, scratch: &Path) -> Result<(), String> {
    std::fs::copy(backup, scratch).map_err(|error| error.to_string())?;
    replace(target, scratch)?;
    if file_sha256(target)? != file_sha256(backup)? {
        return Err("還原後 SHA-256 不符。".into());
    }
    Ok(())
}

fn append_log(stage: &Path, message: &str) {
    // Timestamp is UTC Unix milliseconds; explicitly named so it cannot be
    // mistaken for a timezone-free local datetime.
    let utc_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_millis());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(stage.join("update.log"))
    {
        let _ = writeln!(file, "utcUnixMilliseconds={utc_ms} {message}");
    }
}

pub enum LaunchOutcome {
    Confirmed,
    StillRunning,
}

pub fn apply_files(
    plan: &ApplyPlan,
    launch: impl FnOnce(&Path) -> Result<LaunchOutcome, String>,
) -> Result<(), String> {
    let pending = &plan.pending;
    validate_pending(pending, &pending.target)?;
    if file_sha256(&pending.target)? != plan.original_sha256 {
        return Err("原程式在下載後已改變，取消更新。".into());
    }
    let stage = &pending.stage_directory;
    let main_backup = stage.join("original-main.exe");
    let helper_target = pending
        .target
        .parent()
        .ok_or("目標缺少目錄。")?
        .join("moemusicplayer-updater.exe");
    let helper_backup = stage.join("original-helper.exe");
    if main_backup.exists() || helper_backup.exists() {
        return Err("此更新嘗試已有備份，禁止重複套用。".into());
    }
    std::fs::copy(&pending.target, &main_backup).map_err(|error| error.to_string())?;
    let had_helper = helper_target.exists();
    if had_helper {
        std::fs::copy(&helper_target, &helper_backup).map_err(|error| error.to_string())?;
    }
    let result = (|| {
        replace(
            &pending.target,
            &stage.join(&pending.manifest.executable.name),
        )?;
        verify_file(&pending.target, &pending.manifest.executable)?;
        replace(&helper_target, &stage.join(&pending.manifest.helper.name))?;
        verify_file(&helper_target, &pending.manifest.helper)?;
        launch(&pending.target)
    })();
    if matches!(result, Ok(LaunchOutcome::StillRunning)) {
        return Err(
            "新版程序仍在執行，但尚未確認啟動交接；保留新版與原始備份，不還原或啟動另一個程式。"
                .into(),
        );
    }
    if let Err(error) = result {
        // ReplaceFileW failures can leave partial filesystem state; restore
        // from our independently verified copies rather than assume no change.
        let main_result = restore(
            &pending.target,
            &main_backup,
            &stage.join("rollback-main.exe"),
        );
        let helper_result = if had_helper {
            restore(
                &helper_target,
                &helper_backup,
                &stage.join("rollback-helper.exe"),
            )
        } else if helper_target.exists() {
            std::fs::remove_file(&helper_target).map_err(|error| error.to_string())
        } else {
            Ok(())
        };
        if let Err(rollback_error) = main_result.and(helper_result) {
            return Err(format!(
                "更新失敗：{error}；還原失敗：{rollback_error}。原始備份仍在更新目錄。"
            ));
        }
        return Err(format!("更新失敗，已還原舊版：{error}"));
    }
    Ok(())
}

pub fn helper_main() -> Result<(), String> {
    let plan_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("缺少更新計畫。")?;
    let plan: ApplyPlan = read_json(&plan_path)?;
    let stage = plan
        .pending
        .stage_directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if plan_path
        .canonicalize()
        .map_err(|error| error.to_string())?
        .parent()
        != Some(stage.as_path())
        || plan.nonce.len() != 32
        || !plan.nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("更新計畫位置或許可識別無效。".into());
    }
    validate_pending(&plan.pending, &plan.pending.target)?;
    // OS-held exclusive sharing is released even if the helper crashes; no
    // stale lock file can permanently disable future updates.
    let updates = stage.parent().ok_or("更新目錄不存在。")?;
    let _update_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .share_mode(0)
        .open(updates.join("apply.lock"))
        .map_err(|error| format!("另一個更新或播放器仍占用更新鎖：{error}"))?;
    let wait = wait_for_parent(
        plan.parent_pid,
        plan.parent_started,
        &plan.pending.target,
        Duration::from_secs(u64::from(plan.wait_seconds.clamp(1, 60))),
        || {
            println!("READY");
            let _ = std::io::stdout().flush();
        },
    );
    if let Err(error) = wait {
        append_log(&stage, &error);
        return Err(error);
    }
    if std::fs::read_to_string(stage.join("permit")).map_err(|error| error.to_string())?
        != plan.nonce
    {
        return Err("未收到嚴格保存成功的更新許可；未替換程式。".into());
    }
    let pending_path = updates.join("pending.json");
    // Remove the accepted journal before launch, preventing the new app from
    // recursively applying the old pending update before opening its database.
    if pending_path.exists() {
        std::fs::rename(&pending_path, stage.join("accepted-pending.json"))
            .map_err(|error| error.to_string())?;
    }
    let result = apply_files(&plan, |target| {
        let mut child = std::process::Command::new(target)
            .arg("--update-launched")
            .arg(&stage)
            .spawn()
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if std::fs::read_to_string(stage.join("launch-ready"))
                .ok()
                .as_deref()
                == Some(&plan.nonce)
            {
                return Ok(LaunchOutcome::Confirmed);
            }
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err("新版在啟動交接前結束。".into());
            }
            if Instant::now() >= deadline {
                return Ok(LaunchOutcome::StillRunning);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    });
    match result {
        Ok(()) => {
            append_log(
                &stage,
                "updated and launched before database initialization",
            );
            Ok(())
        }
        Err(error) => {
            append_log(&stage, &error);
            // If a process still locks the restored file, launch fails plainly;
            // the helper never kills either the parent or another app instance.
            if file_sha256(&plan.pending.target).ok().as_deref() == Some(&plan.original_sha256) {
                std::process::Command::new(&plan.pending.target)
                    .spawn()
                    .map_err(|launch_error| format!("{error}；舊版重新啟動失敗：{launch_error}"))?;
            }
            Err(error)
        }
    }
}
