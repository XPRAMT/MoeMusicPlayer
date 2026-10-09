//! Portable Windows update metadata and staging. All locations are owned by
//! this service; the renderer can select timing, never a URL or filesystem path.
pub use moe_update::{FileIdentity, PendingUpdate, ReleaseManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};

const REPOSITORY: &str = "https://github.com/XPRAMT/MoeMusicPlayer";
const AUTHOR: &str = "https://github.com/XPRAMT";
const RELEASE_API: &str = "https://api.github.com/repos/XPRAMT/MoeMusicPlayer/releases/latest";
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const MANIFEST_NAME: &str = "moemusicplayer-update.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credit {
    pub name: String,
    pub version: String,
    pub license: String,
    pub repository_url: String,
    pub purpose: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    name: &'static str,
    author: &'static str,
    author_url: &'static str,
    repository_url: &'static str,
    build_version: &'static str,
    build_timestamp_utc: &'static str,
    git_commit: &'static str,
    architecture: &'static str,
    credits: Vec<Credit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestBuild {
    pub build_id: String,
    pub version: String,
    pub published_at: String,
    pub notes: String,
    pub download_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub status: String,
    pub current_build_id: String,
    pub latest: Option<LatestBuild>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Deserialize)]
struct Release {
    html_url: String,
    published_at: String,
    body: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Clone)]
struct AvailableRelease {
    manifest: ReleaseManifest,
    executable: Asset,
    helper: Asset,
    release_url: String,
    published_at: String,
    notes: String,
}

pub struct UpdateService {
    state: Mutex<UpdateState>,
    available: Mutex<Option<AvailableRelease>>,
    operation: tokio::sync::Mutex<()>,
}

impl Default for UpdateService {
    fn default() -> Self {
        Self {
            state: Mutex::new(UpdateState {
                status: "idle".into(),
                current_build_id: current_build_id(),
                latest: None,
                downloaded_bytes: 0,
                total_bytes: None,
                error: None,
            }),
            available: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
        }
    }
}

fn current_build_id() -> String {
    format!(
        "{}@{}",
        env!("MOE_BUILD_TIMESTAMP_UTC"),
        env!("MOE_GIT_COMMIT")
    )
}

fn credits() -> Vec<Credit> {
    serde_json::from_str(include_str!("../../licenses/credits.json"))
        .expect("validated embedded credits")
}

#[tauri::command]
pub fn about_get_info() -> AboutInfo {
    AboutInfo {
        name: "MoeMusicPlayer",
        author: "XPRAMT",
        author_url: AUTHOR,
        repository_url: REPOSITORY,
        build_version: env!("MOE_BUILD_VERSION"),
        build_timestamp_utc: env!("MOE_BUILD_TIMESTAMP_UTC"),
        git_commit: env!("MOE_GIT_COMMIT"),
        architecture: std::env::consts::ARCH,
        credits: credits(),
    }
}

#[tauri::command]
pub fn update_get_state(service: State<'_, UpdateService>) -> UpdateState {
    service
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

pub(crate) fn publish(
    app: &AppHandle,
    service: &UpdateService,
    change: impl FnOnce(&mut UpdateState),
) -> UpdateState {
    let state = {
        let mut state = service
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        change(&mut state);
        state.clone()
    };
    let _ = app.emit("update-state-changed", &state);
    state
}

fn failed(app: &AppHandle, service: &UpdateService, error: String) -> UpdateState {
    publish(app, service, |state| {
        state.status = "error".into();
        state.error = Some(error);
    })
}

#[tauri::command]
pub fn update_ignore(app: AppHandle, service: State<'_, UpdateService>) -> UpdateState {
    // Ignore this notification only. A manual/next-start check can notify again.
    publish(&app, &service, |state| state.status = "idle".into())
}

fn trusted_download_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str() == Some("github.com")
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none()
        && parsed
            .path()
            .starts_with("/XPRAMT/MoeMusicPlayer/releases/download/")
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("MoeMusicPlayer-updater")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5
                || url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "api.github.com"
                            | "objects.githubusercontent.com"
                            | "release-assets.githubusercontent.com"
                    )
                )
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
            {
                attempt.error("update redirect outside allowed GitHub hosts")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| error.to_string())
}

async fn bounded_bytes(client: &reqwest::Client, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response.content_length().is_some_and(|size| size > limit) {
        return Err("更新回應超過大小上限。".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            return Err("更新回應超過大小上限。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_manifest(manifest: &ReleaseManifest) -> Result<(), String> {
    if manifest.schema_version != 1 || manifest.architecture != "windows-x86_64" {
        return Err("此更新版本或架構不受支援。".into());
    }
    if !valid_utc(&manifest.build_timestamp_utc)
        || manifest.git_commit.len() != 40
        || !manifest
            .git_commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("更新建置識別無效。".into());
    }
    for (file, name, limit) in [
        (&manifest.executable, "moemusicplayer.exe", MAX_DOWNLOAD),
        (
            &manifest.helper,
            "moemusicplayer-updater.exe",
            16 * 1024 * 1024,
        ),
    ] {
        if file.name != name || !valid_hash(&file.sha256) || file.size == 0 || file.size > limit {
            return Err("更新檔案清單無效。".into());
        }
    }
    Ok(())
}

fn valid_utc(value: &str) -> bool {
    let bytes = value.as_bytes();
    let syntax = bytes.len() == 24
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'.'
        && bytes[23] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 23) || byte.is_ascii_digit()
        });
    if !syntax {
        return false;
    }
    let number = |range: std::ops::Range<usize>| value[range].parse::<u32>().unwrap_or(0);
    let year = number(0..4);
    let month = number(5..7);
    let day = number(8..10);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let month_days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    year >= 1970
        && day != 0
        && day <= month_days
        && number(11..13) < 24
        && number(14..16) < 60
        && number(17..19) < 60
}

fn matching_asset(release: &Release, file: &FileIdentity) -> Result<Asset, String> {
    let found: Vec<_> = release
        .assets
        .iter()
        .filter(|asset| asset.name == file.name)
        .collect();
    if found.len() != 1
        || found[0].size != file.size
        || !trusted_download_url(&found[0].browser_download_url)
    {
        return Err("GitHub 更新資產不符合清單。".into());
    }
    Ok(found[0].clone())
}

async fn check_release() -> Result<Option<AvailableRelease>, String> {
    let client = client()?;
    let response = client
        .get(RELEASE_API)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| format!("無法檢查更新：{error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let mut response = response
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() + chunk.len() > 512 * 1024 {
            return Err("Release 回應過大。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let release: Release = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if !release
        .html_url
        .starts_with(&format!("{REPOSITORY}/releases/tag/"))
    {
        return Err("Release 來源無效。".into());
    }
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == MANIFEST_NAME)
        .ok_or("Release 缺少可驗證的更新清單。")?;
    if !trusted_download_url(&asset.browser_download_url) {
        return Err("更新清單來源無效。".into());
    }
    let hash = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| valid_hash(hash))
        .ok_or("GitHub 未提供更新清單 SHA-256 digest；為保護完整性，無法自動更新。")?;
    let bytes = bounded_bytes(&client, &asset.browser_download_url, 64 * 1024).await?;
    if bytes.len() as u64 != asset.size || sha256(&bytes) != hash.to_ascii_lowercase() {
        return Err("更新清單 SHA-256 驗證失敗。".into());
    }
    let manifest: ReleaseManifest =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    validate_manifest(&manifest)?;
    let executable = matching_asset(&release, &manifest.executable)?;
    let helper = matching_asset(&release, &manifest.helper)?;
    Ok(Some(AvailableRelease {
        manifest,
        executable,
        helper,
        release_url: release.html_url,
        published_at: release.published_at,
        notes: release.body.unwrap_or_default(),
    }))
}

#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    service: State<'_, UpdateService>,
) -> Result<UpdateState, String> {
    let Ok(_operation) = service.operation.try_lock() else {
        return Ok(service
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone());
    };
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Ok(publish(&app, &service, |state| {
            state.status = "unsupported".into();
            state.error = Some("自動更新目前僅支援 Windows x64 便攜版。".into());
        }));
    }
    publish(&app, &service, |state| {
        state.status = "checking".into();
        state.error = None;
    });
    *service
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    match check_release().await {
        Ok(None) => Ok(publish(&app, &service, |state| {
            state.status = "unpublished".into();
            state.latest = None;
        })),
        Ok(Some(release)) => {
            let is_new =
                release.manifest.build_timestamp_utc.as_str() > env!("MOE_BUILD_TIMESTAMP_UTC");
            let state = publish(&app, &service, |state| {
                state.status = if is_new { "available" } else { "current" }.into();
                state.latest = Some(LatestBuild {
                    build_id: format!(
                        "{}@{}",
                        release.manifest.build_timestamp_utc, release.manifest.git_commit
                    ),
                    version: release.manifest.version.clone(),
                    published_at: release.published_at.clone(),
                    notes: release.notes.clone(),
                    download_bytes: release.manifest.executable.size + release.manifest.helper.size,
                });
            });
            *service
                .available
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(release);
            Ok(state)
        }
        Err(error) => Ok(failed(&app, &service, error)),
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateMode {
    RestartNow,
    NextLaunch,
}

#[cfg(target_os = "windows")]
fn updates_directory() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let data = crate::windows_data_directory::user_data_dir_for_executable(&exe)
        .map_err(|error| error.to_string())?;
    let updates = data.join("updates");
    std::fs::create_dir_all(&updates).map_err(|error| error.to_string())?;
    let canonical = updates.canonicalize().map_err(|error| error.to_string())?;
    if canonical.parent()
        != Some(
            data.canonicalize()
                .map_err(|error| error.to_string())?
                .as_path(),
        )
    {
        return Err("更新目錄不可指向 UserData 之外。".into());
    }
    Ok(canonical)
}

async fn download_file(
    app: Option<&AppHandle>,
    service: &UpdateService,
    client: &reqwest::Client,
    asset: &Asset,
    identity: &FileIdentity,
    path: &Path,
) -> Result<(), String> {
    let mut response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|size| size != identity.size)
    {
        return Err("更新下載長度不符。".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let mut size = 0u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        size += chunk.len() as u64;
        if size > identity.size {
            return Err("更新下載超過清單大小。".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk).map_err(|error| error.to_string())?;
        if let Some(app) = app {
            publish(app, service, |state| {
                state.downloaded_bytes += chunk.len() as u64
            });
        } else {
            service
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .downloaded_bytes += chunk.len() as u64;
        }
    }
    file.sync_all().map_err(|error| error.to_string())?;
    if size != identity.size
        || format!("{:x}", hash.finalize()) != identity.sha256.to_ascii_lowercase()
    {
        return Err("更新檔案 SHA-256 驗證失敗。".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
async fn stage_release(
    app: Option<&AppHandle>,
    service: &UpdateService,
    release: AvailableRelease,
) -> Result<PendingUpdate, String> {
    let directory = updates_directory()?;
    let stage_directory = directory.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&stage_directory).map_err(|error| error.to_string())?;
    let result = async {
        let client = client()?;
        download_file(
            app,
            service,
            &client,
            &release.executable,
            &release.manifest.executable,
            &stage_directory.join(&release.manifest.executable.name),
        )
        .await?;
        download_file(
            app,
            service,
            &client,
            &release.helper,
            &release.manifest.helper,
            &stage_directory.join(&release.manifest.helper.name),
        )
        .await?;
        let pending = PendingUpdate {
            manifest: release.manifest,
            stage_directory: stage_directory.clone(),
            target: std::env::current_exe().map_err(|error| error.to_string())?,
        };
        validate_for_apply(&pending)?;
        save_pending(&directory, &pending)?;
        Ok(pending)
    }
    .await;
    if result.is_err() {
        // This nonce directory was created by this invocation under the
        // canonical, validated update root; no user music/settings are inside.
        let _ = std::fs::remove_dir_all(&stage_directory);
    }
    result
}

/// Diagnostic CLI uses exactly the same trusted Release metadata and staging
/// paths as the UI. There is deliberately no configurable server or URL.
#[cfg(target_os = "windows")]
pub async fn diagnostic_check(stage_for_next_launch: bool) -> Result<UpdateState, String> {
    let service = UpdateService::default();
    let Some(release) = check_release().await? else {
        let mut state = service
            .state
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.status = "unpublished".into();
        return Ok(state);
    };
    let latest = LatestBuild {
        build_id: format!(
            "{}@{}",
            release.manifest.build_timestamp_utc, release.manifest.git_commit
        ),
        version: release.manifest.version.clone(),
        published_at: release.published_at.clone(),
        notes: release.notes.clone(),
        download_bytes: release.manifest.executable.size + release.manifest.helper.size,
    };
    let newer = release.manifest.build_timestamp_utc.as_str() > env!("MOE_BUILD_TIMESTAMP_UTC");
    if stage_for_next_launch {
        if !newer {
            return Err("此版本不比目前建置新。".into());
        }
        stage_release(None, &service, release).await?;
    }
    let mut state = service
        .state
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.status = if stage_for_next_launch {
        "deferred"
    } else if newer {
        "available"
    } else {
        "current"
    }
    .into();
    state.latest = Some(latest);
    Ok(state)
}

#[tauri::command]
pub async fn update_download(
    app: AppHandle,
    service: State<'_, UpdateService>,
    mode: UpdateMode,
) -> Result<UpdateState, String> {
    let Ok(_operation) = service.operation.try_lock() else {
        return Err("更新作業已在執行。".into());
    };
    let release = service
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or("請先檢查更新。")?;
    if release.manifest.build_timestamp_utc.as_str() <= env!("MOE_BUILD_TIMESTAMP_UTC") {
        return Err("此版本不比目前建置新。".into());
    }
    publish(&app, &service, |state| {
        state.status = "downloading".into();
        state.downloaded_bytes = 0;
        state.total_bytes = Some(release.manifest.executable.size + release.manifest.helper.size);
        state.error = None;
    });
    #[cfg(target_os = "windows")]
    let result = async {
        let pending = stage_release(Some(&app), &service, release).await?;
        match mode {
            UpdateMode::NextLaunch => Ok(publish(&app, &service, |state| {
                state.status = "deferred".into()
            })),
            UpdateMode::RestartNow => {
                let app = app.clone();
                let result = tauri::async_runtime::spawn_blocking(move || {
                    crate::apply_downloaded_update(&app, &pending)
                })
                .await
                .map_err(|error| error.to_string())?;
                if result.is_err() {
                    let directory = updates_directory()?;
                    let journal = directory.join("pending.json");
                    // The operation mutex still owns this attempt. A failed
                    // strict save must not silently turn into next-start apply.
                    if journal.exists() {
                        std::fs::remove_file(journal).map_err(|error| error.to_string())?;
                    }
                }
                result
            }
        }
    }
    .await;
    #[cfg(not(target_os = "windows"))]
    let result: Result<UpdateState, String> = {
        let _ = mode;
        Err("此平台不支援自動更新。".into())
    };
    match result {
        Ok(state) => Ok(state),
        Err(error) => Ok(failed(&app, &service, error)),
    }
}

#[cfg(target_os = "windows")]
pub fn save_pending(directory: &Path, pending: &PendingUpdate) -> Result<(), String> {
    let temporary = directory.join("pending.tmp");
    let mut file = std::fs::File::create(&temporary).map_err(|error| error.to_string())?;
    file.write_all(&serde_json::to_vec(pending).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    std::fs::rename(temporary, directory.join("pending.json")).map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn validate_for_apply(pending: &PendingUpdate) -> Result<(), String> {
    validate_manifest(&pending.manifest)?;
    moe_update::validate_pending(
        pending,
        &std::env::current_exe().map_err(|error| error.to_string())?,
    )
}

#[cfg(target_os = "windows")]
pub(crate) fn launch_helper(pending: &PendingUpdate) -> Result<moe_update::ApplyPlan, String> {
    use std::io::BufRead;
    use std::os::windows::process::CommandExt;
    validate_for_apply(pending)?;
    let plan = moe_update::ApplyPlan {
        pending: pending.clone(),
        parent_pid: std::process::id(),
        parent_started: moe_update::windows::process_started(std::process::id())?,
        original_sha256: moe_update::file_sha256(&pending.target)?,
        nonce: uuid::Uuid::new_v4().simple().to_string(),
        wait_seconds: 60,
    };
    let stage = &pending.stage_directory;
    let plan_path = stage.join("apply-plan.json");
    moe_update::write_json(&plan_path, &plan)?;
    let running_helper = stage.join("running-helper.exe");
    std::fs::copy(stage.join(&pending.manifest.helper.name), &running_helper)
        .map_err(|error| error.to_string())?;
    moe_update::verify_file(&running_helper, &pending.manifest.helper)?;
    let mut child = std::process::Command::new(running_helper)
        .arg(plan_path)
        .creation_flags(0x0800_0000)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("無法啟動更新輔助程式：{error}"))?;
    let output = child.stdout.take().ok_or("更新輔助程式無法建立交接。")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = std::io::BufReader::new(output)
            .read_line(&mut line)
            .map(|_| line);
        let _ = sender.send(result);
    });
    let line = receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "更新輔助程式未及時確認；已取消更新。")?
        .map_err(|error| error.to_string())?;
    if line.trim() != "READY" {
        return Err("更新輔助程式未確認父程序身分；已取消更新。".into());
    }
    Ok(plan)
}

#[cfg(target_os = "windows")]
pub(crate) fn grant_permit(plan: &moe_update::ApplyPlan) -> Result<(), String> {
    let mut permit = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(plan.pending.stage_directory.join("permit"))
        .map_err(|error| error.to_string())?;
    permit
        .write_all(plan.nonce.as_bytes())
        .map_err(|error| error.to_string())?;
    permit.sync_all().map_err(|error| error.to_string())
}

/// Called by main before Tauri, settings, or SQLite initialize.
#[cfg(target_os = "windows")]
pub fn finish_pending_before_app() -> bool {
    let result = (|| -> Result<bool, String> {
        let directory = updates_directory()?;
        let path = directory.join("pending.json");
        if !path.exists() {
            return Ok(false);
        }
        let pending: PendingUpdate = moe_update::read_json(&path)?;
        validate_for_apply(&pending)?;
        let plan = launch_helper(&pending)?;
        grant_permit(&plan)?;
        Ok(true)
    })();
    match result {
        Ok(should_exit) => should_exit,
        Err(error) => {
            eprintln!("下次啟動更新未套用：{error}");
            false
        }
    }
}

#[cfg(target_os = "windows")]
pub fn acknowledge_updated_launch(stage: &Path) -> Result<(), String> {
    let actual = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = updates_directory()?;
    let stage = stage.canonicalize().map_err(|error| error.to_string())?;
    if stage.parent() != Some(directory.as_path()) {
        return Err("更新交接目錄無效。".into());
    }
    let plan: moe_update::ApplyPlan = moe_update::read_json(&stage.join("apply-plan.json"))?;
    if plan
        .pending
        .target
        .canonicalize()
        .map_err(|error| error.to_string())?
        != actual.canonicalize().map_err(|error| error.to_string())?
    {
        return Err("更新交接目標不符。".into());
    }
    moe_update::verify_file(&actual, &plan.pending.manifest.executable)?;
    let mut ready = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(stage.join("launch-ready"))
        .map_err(|error| error.to_string())?;
    ready
        .write_all(plan.nonce.as_bytes())
        .map_err(|error| error.to_string())?;
    ready.sync_all().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn app_open_external_url(app: AppHandle, url: String) -> Result<(), String> {
    let service = app.state::<UpdateService>();
    let release_url = service
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(|release| release.release_url.clone());
    if url != AUTHOR
        && url != REPOSITORY
        && release_url.as_deref() != Some(&url)
        && !credits().iter().any(|credit| credit.repository_url == url)
    {
        return Err("此連結不在官方來源白名單。".into());
    }
    if reqwest::Url::parse(&url)
        .ok()
        .is_none_or(|url| url.scheme() != "https")
    {
        return Err("只允許 HTTPS 官方連結。".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                window: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        let url: Vec<u16> = std::ffi::OsStr::new(&url)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result <= 32 {
            return Err(format!("無法開啟官方連結（{result}）。"));
        }
        Ok(())
    }
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_media_index::MediaIndexExt;
        app.media_index()
            .open_official_url(&url)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    Err("此平台目前不支援由播放器開啟外部連結。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_urls_reject_foreign_hosts_paths_and_schemes() {
        assert!(trusted_download_url(
            "https://github.com/XPRAMT/MoeMusicPlayer/releases/download/v1/moemusicplayer.exe"
        ));
        for url in [
            "http://github.com/XPRAMT/MoeMusicPlayer/releases/download/v1/a",
            "https://github.com/other/repo/releases/download/v1/a",
            "https://github.com.evil.test/XPRAMT/MoeMusicPlayer/releases/download/v1/a",
            "https://user@github.com/XPRAMT/MoeMusicPlayer/releases/download/v1/a",
            "file:///C:/app.exe",
        ] {
            assert!(!trusted_download_url(url), "{url}");
        }
    }
    #[test]
    fn build_order_retains_same_day_precision_and_utc_requirement() {
        assert!(valid_utc("2026-10-09T01:02:03.456Z"));
        assert!(!valid_utc("2026-10-09T01:02:03"));
        assert!("2026-10-09T01:02:03.457Z" > "2026-10-09T01:02:03.456Z");
    }
    #[test]
    fn credits_are_embedded_actual_dependencies_and_exclude_proprietary_filter() {
        let credits = credits();
        assert!(credits
            .iter()
            .any(|credit| credit.name == "opencc-js" && credit.license == "MIT AND Apache-2.0"));
        assert!(credits
            .iter()
            .any(|credit| credit.name == "AndroidX Media3"));
        assert!(!credits.iter().any(|credit| credit.name.contains("Sony")));
    }
}
