//! Windows and Android updates from the latest GitHub Release (ADR 0022). An update is identified only by the
//! version in the Release tag (`v26.10.19`, the build date); the file is verified against the SHA-256 digest GitHub
//! reports for the asset. All locations are owned by this service; the renderer can select timing, never a URL or
//! path.
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
const DOWNLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// The two assets of a Release: the Windows package (a zip holding `moemusicplayer.exe`) and the Android APK.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const WINDOWS_ZIP_NAME: &str = "moemusicplayer-windows-x86_64.zip";
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const WINDOWS_EXE_NAME: &str = "moemusicplayer.exe";
const ANDROID_APK_NAME: &str = "moemusicplayer-android-arm64.apk";

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
    tag_name: String,
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

/// A file of a Release: its name, GitHub's SHA-256 digest and its size.
#[derive(Clone, Debug)]
pub struct FileIdentity {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

/// The latest Release's file for this platform.
#[derive(Clone)]
struct AvailableRelease {
    version: String,
    asset: Asset,
    identity: FileIdentity,
    release_url: String,
    published_at: String,
    notes: String,
}

pub struct UpdateService {
    state: Mutex<UpdateState>,
    available: Mutex<Option<AvailableRelease>>,
    /// Verified APK already downloaded for `available` (retry after the install prompt).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    android_apk: Mutex<Option<PathBuf>>,
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
            android_apk: Mutex::new(None),
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

/// The update version the user chose Ignore for (settings.json), if any.
fn ignored_update_version(app: &AppHandle) -> Option<String> {
    app.try_state::<crate::AppState>()?
        .settings
        .snapshot()
        .ok()?
        .ignored_update_version
}

fn remember_ignored_update_version(app: &AppHandle, version: &str) {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return;
    };
    if let Err(error) = state.settings.update(|settings| {
        settings.ignored_update_version = Some(version.to_owned());
        Ok(())
    }) {
        eprintln!("無法記錄已忽略的更新版本：{error}");
    }
}

/// The status of a finished check: a newer version is `available`, unless the user chose Ignore for exactly this
/// version and the check was not asked for by hand (`ignored`: no prompt, shown only in Settings → About).
fn check_status(is_new: bool, ignored: bool, manual: bool) -> &'static str {
    if !is_new {
        "current"
    } else if ignored && !manual {
        "ignored"
    } else {
        "available"
    }
}

#[tauri::command]
pub fn update_ignore(app: AppHandle, service: State<'_, UpdateService>) -> UpdateState {
    // Ignore this version: it is not offered again on startup, only when the user checks for updates by hand.
    let version = service
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(|release| release.version.clone());
    match version {
        Some(version) => {
            remember_ignored_update_version(&app, &version);
            publish(&app, &service, |state| state.status = "ignored".into())
        }
        None => publish(&app, &service, |state| state.status = "idle".into()),
    }
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

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// `v26.10.19` or `26.10.19` (the build date) -> (26, 10, 19).
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut next = || -> Option<u32> {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 4 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    matches!(
        (parse_version(latest), parse_version(current)),
        (Some(latest), Some(current)) if latest > current
    )
}

/// The Release's file named `asset_name`, or None when this Release has no file for the platform. GitHub's SHA-256
/// digest of the asset is the integrity check, so a Release without it cannot be installed automatically.
fn available_release(
    release: Release,
    asset_name: &str,
) -> Result<Option<AvailableRelease>, String> {
    let version = release
        .tag_name
        .strip_prefix('v')
        .filter(|version| parse_version(version).is_some())
        .ok_or("Release 版本標籤無效。")?
        .to_owned();
    let found: Vec<_> = release
        .assets
        .iter()
        .filter(|asset| asset.name == asset_name)
        .collect();
    let asset = match found.as_slice() {
        [] => return Ok(None),
        [asset] => (*asset).clone(),
        _ => return Err("GitHub 更新資產重複。".into()),
    };
    if asset.size == 0
        || asset.size > MAX_DOWNLOAD
        || !trusted_download_url(&asset.browser_download_url)
    {
        return Err("GitHub 更新資產無效。".into());
    }
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| valid_hash(hash))
        .ok_or("GitHub 未提供更新檔案 SHA-256 digest；為保護完整性，無法自動更新。")?
        .to_ascii_lowercase();
    Ok(Some(AvailableRelease {
        version,
        identity: FileIdentity {
            name: asset.name.clone(),
            sha256,
            size: asset.size,
        },
        asset,
        release_url: release.html_url,
        published_at: release.published_at,
        notes: release.body.unwrap_or_default(),
    }))
}

/// The fixed repository's public latest Release, or None when nothing is published.
async fn latest_release(client: &reqwest::Client) -> Result<Option<Release>, String> {
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
    Ok(Some(release))
}

async fn check_release(asset_name: &str) -> Result<Option<AvailableRelease>, String> {
    let client = client()?;
    match latest_release(&client).await? {
        Some(release) => available_release(release, asset_name),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    service: State<'_, UpdateService>,
    manual: Option<bool>,
) -> Result<UpdateState, String> {
    let manual = manual.unwrap_or(false);
    let Ok(_operation) = service.operation.try_lock() else {
        return Ok(service
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone());
    };
    let asset_name = if cfg!(all(target_os = "android", target_arch = "aarch64")) {
        ANDROID_APK_NAME
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        WINDOWS_ZIP_NAME
    } else {
        return Ok(publish(&app, &service, |state| {
            state.status = "unsupported".into();
            state.error = Some("自動更新目前僅支援 Windows x64 便攜版與 Android arm64。".into());
        }));
    };
    publish(&app, &service, |state| {
        state.status = "checking".into();
        state.error = None;
    });
    *service
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    match check_release(asset_name).await {
        Ok(None) => Ok(publish(&app, &service, |state| {
            state.status = "unpublished".into();
            state.latest = None;
        })),
        Ok(Some(release)) => {
            let is_new = is_newer_version(&release.version, env!("MOE_BUILD_VERSION"));
            let ignored = ignored_update_version(&app).as_deref() == Some(release.version.as_str());
            let state = publish(&app, &service, |state| {
                state.status = check_status(is_new, ignored, manual).into();
                state.latest = Some(LatestBuild {
                    build_id: release.version.clone(),
                    version: release.version.clone(),
                    published_at: release.published_at.clone(),
                    notes: release.notes.clone(),
                    download_bytes: release.identity.size,
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
    let mut response = wait_for_download_progress(
        client
            .get(&asset.browser_download_url)
            .timeout(DOWNLOAD_TOTAL_TIMEOUT)
            .send(),
        DOWNLOAD_IDLE_TIMEOUT,
    )
    .await?
    .map_err(download_error)?
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
    while let Some(chunk) = wait_for_download_progress(response.chunk(), DOWNLOAD_IDLE_TIMEOUT)
        .await?
        .map_err(download_error)?
    {
        size += chunk.len() as u64;
        if size > identity.size {
            return Err("更新下載超過預期大小。".into());
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

async fn wait_for_download_progress<T>(
    work: impl std::future::Future<Output = T>,
    idle_timeout: Duration,
) -> Result<T, String> {
    tokio::time::timeout(idle_timeout, work)
        .await
        .map_err(|_| "更新下載已連續 30 秒沒有收到資料，請稍後重試。".into())
}

fn download_error(error: reqwest::Error) -> String {
    if error.is_connect() && error.is_timeout() {
        "更新下載連線未能在 10 秒內建立，請稍後重試。".into()
    } else if error.is_timeout() {
        "更新下載已超過 30 分鐘總上限，請稍後重試。".into()
    } else {
        format!("更新下載中斷：{}", error.without_url())
    }
}

/// A downloaded release: the verified zip is unpacked into the update stage, ready to be installed over `target`.
#[cfg(target_os = "windows")]
#[derive(Clone)]
pub struct StagedUpdate {
    pub stage_directory: PathBuf,
    pub executable: PathBuf,
    pub target: PathBuf,
}

#[cfg(target_os = "windows")]
async fn stage_release(
    app: Option<&AppHandle>,
    service: &UpdateService,
    release: AvailableRelease,
) -> Result<StagedUpdate, String> {
    let directory = updates_directory()?;
    let stage_directory = directory.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&stage_directory).map_err(|error| error.to_string())?;
    let result = tokio::time::timeout(DOWNLOAD_TOTAL_TIMEOUT, async {
        let client = client()?;
        let archive = stage_directory.join(WINDOWS_ZIP_NAME);
        download_file(
            app,
            service,
            &client,
            &release.asset,
            &release.identity,
            &archive,
        )
        .await?;
        let executable = stage_directory.join(WINDOWS_EXE_NAME);
        extract_executable(&archive, &executable)?;
        let staged = StagedUpdate {
            stage_directory: stage_directory.clone(),
            executable,
            target: std::env::current_exe().map_err(|error| error.to_string())?,
        };
        validate_for_apply(&staged)?;
        Ok(staged)
    })
    .await
    .unwrap_or_else(|_| Err("更新下載已超過 30 分鐘總上限，請稍後重試。".into()));
    if result.is_err() {
        // This nonce directory was created by this invocation under the
        // canonical, validated update root; no user music/settings are inside.
        let _ = std::fs::remove_dir_all(&stage_directory);
    }
    result
}

/// Unpacks `moemusicplayer.exe` from the (digest-verified) release zip: that one entry by exact name, bounded in size.
#[cfg(target_os = "windows")]
fn extract_executable(archive: &Path, output: &Path) -> Result<(), String> {
    use std::io::Read;
    let file = std::fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| format!("更新壓縮檔無效：{error}"))?;
    let mut entry = zip
        .by_name(WINDOWS_EXE_NAME)
        .map_err(|_| "更新壓縮檔缺少 moemusicplayer.exe。".to_owned())?;
    let size = entry.size();
    if size == 0 || size > MAX_DOWNLOAD {
        return Err("更新壓縮檔內的 moemusicplayer.exe 大小無效。".into());
    }
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|error| error.to_string())?;
    let copied = std::io::copy(&mut entry.by_ref().take(size + 1), &mut out)
        .map_err(|error| error.to_string())?;
    if copied != size {
        return Err("更新壓縮檔解壓縮長度不符。".into());
    }
    out.sync_all().map_err(|error| error.to_string())
}

/// Diagnostic CLI uses exactly the same trusted Release metadata as the UI. There is deliberately no configurable
/// server or URL.
#[cfg(target_os = "windows")]
pub async fn diagnostic_check() -> Result<UpdateState, String> {
    let service = UpdateService::default();
    let mut state = service
        .state
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(release) = check_release(WINDOWS_ZIP_NAME).await? else {
        state.status = "unpublished".into();
        return Ok(state);
    };
    state.status = if is_newer_version(&release.version, env!("MOE_BUILD_VERSION")) {
        "available"
    } else {
        "current"
    }
    .into();
    state.latest = Some(LatestBuild {
        build_id: release.version.clone(),
        version: release.version,
        published_at: release.published_at,
        notes: release.notes,
        download_bytes: release.identity.size,
    });
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
    #[cfg(target_os = "android")]
    {
        // One flow on Android: download + verify, then the system installer (mode is ignored).
        let _ = mode;
        Ok(match android_download_and_install(&app, &service).await {
            Ok(state) => state,
            Err(error) => failed(&app, &service, error),
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let release = service
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or("請先檢查更新。")?;
        if !is_newer_version(&release.version, env!("MOE_BUILD_VERSION")) {
            return Err("此版本不比目前版本新。".into());
        }
        publish(&app, &service, |state| {
            state.status = "downloading".into();
            state.downloaded_bytes = 0;
            state.total_bytes = Some(release.identity.size);
            state.error = None;
        });
        #[cfg(target_os = "windows")]
        let result = async {
            let staged = stage_release(Some(&app), &service, release).await?;
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                crate::install_downloaded_update(
                    &app,
                    &staged,
                    matches!(mode, UpdateMode::RestartNow),
                )
            })
            .await
            .map_err(|error| error.to_string())?
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
}

/// Download the APK into the app-private `cache/updates` (exposed only to the system installer
/// through the plugin's FileProvider), verify size + SHA-256 against the manifest, then hand it to
/// the installer. Without "install unknown apps" the settings page opens instead
/// (`installPermission`); pressing the button again reuses the verified download.
#[cfg(target_os = "android")]
async fn android_download_and_install(
    app: &AppHandle,
    service: &UpdateService,
) -> Result<UpdateState, String> {
    use tauri_plugin_media_index::MediaIndexExt;

    let release = service
        .available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or("請先檢查更新。")?;
    if !is_newer_version(&release.version, env!("MOE_BUILD_VERSION")) {
        return Err("此版本不比目前版本新。".into());
    }
    let cached = service
        .android_apk
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .filter(|path| {
            std::fs::metadata(path).is_ok_and(|metadata| metadata.len() == release.identity.size)
        });
    let path = match cached {
        Some(path) => path,
        None => {
            publish(app, service, |state| {
                state.status = "downloading".into();
                state.downloaded_bytes = 0;
                state.total_bytes = Some(release.identity.size);
                state.error = None;
            });
            let directory_app = app.clone();
            let directory = tauri::async_runtime::spawn_blocking(move || {
                directory_app.media_index().update_download_directory()
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
            // Only APKs this service downloaded live here; drop stale ones before a new download.
            if let Ok(entries) = std::fs::read_dir(&directory) {
                for entry in entries.flatten() {
                    if entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "apk")
                    {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
            let path = directory.join(format!(
                "{}-{ANDROID_APK_NAME}",
                uuid::Uuid::new_v4().simple()
            ));
            let client = client()?;
            let downloaded = tokio::time::timeout(
                DOWNLOAD_TOTAL_TIMEOUT,
                download_file(
                    Some(app),
                    service,
                    &client,
                    &release.asset,
                    &release.identity,
                    &path,
                ),
            )
            .await
            .unwrap_or_else(|_| Err("更新下載已超過 30 分鐘總上限，請稍後重試。".into()));
            if let Err(error) = downloaded {
                let _ = std::fs::remove_file(&path);
                return Err(error);
            }
            *service
                .android_apk
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(path.clone());
            path
        }
    };
    let install_app = app.clone();
    let apk_path = path.to_string_lossy().into_owned();
    let status = tauri::async_runtime::spawn_blocking(move || {
        install_app.media_index().install_apk(&apk_path)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| format!("無法開啟系統安裝程式：{error}"))?;
    Ok(publish(app, service, |state| {
        state.status = if status == "permission" {
            "installPermission"
        } else {
            "installing"
        }
        .into();
        state.error = None;
    }))
}

#[cfg(target_os = "windows")]
pub(crate) fn validate_for_apply(staged: &StagedUpdate) -> Result<(), String> {
    let actual = std::env::current_exe().map_err(|error| error.to_string())?;
    let target = staged
        .target
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if target != actual.canonicalize().map_err(|error| error.to_string())?
        || target.file_name().and_then(|name| name.to_str()) != Some(WINDOWS_EXE_NAME)
    {
        return Err("更新目標必須是目前的 moemusicplayer.exe。".into());
    }
    let parent = target.parent().ok_or("更新目標缺少目錄。")?;
    let updates = parent
        .join("UserData")
        .join("updates")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if updates.parent().and_then(Path::parent) != Some(parent) {
        return Err("更新資料目錄不可指向應用程式目錄之外。".into());
    }
    let stage = staged
        .stage_directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if stage.parent() != Some(updates.as_path()) {
        return Err("更新暫存必須直接位於 UserData/updates。".into());
    }
    let executable = staged
        .executable
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if executable.parent() != Some(stage.as_path()) {
        return Err("更新檔案不可使用外部連結。".into());
    }
    let mut header = [0u8; 2];
    std::io::Read::read_exact(
        &mut std::fs::File::open(&executable).map_err(|error| error.to_string())?,
        &mut header,
    )
    .map_err(|error| error.to_string())?;
    if &header != b"MZ" {
        return Err("更新的 moemusicplayer.exe 不是有效的執行檔。".into());
    }
    Ok(())
}

/// Installs a staged release by renaming: the running image keeps its (renamed) file and the new file takes the
/// original name, so the new version starts on the next launch. A backup of the previous file stays beside it
/// (`.moemusicplayer.previous.<id>.bak`); only the newest one is kept. Nothing is replaced unless the copy was verified,
/// and a failure puts the original name back.
#[cfg(target_os = "windows")]
pub(crate) fn install_in_place(staged: &StagedUpdate) -> Result<(), String> {
    validate_for_apply(staged)?;
    let directory = staged
        .target
        .parent()
        .ok_or("更新目標缺少目錄。")?
        .to_path_buf();
    install_files(
        &[(staged.executable.clone(), staged.target.clone())],
        &directory,
    )?;
    // Older installs carried a separate updater executable; nothing uses it any more.
    let _ = std::fs::remove_file(directory.join("moemusicplayer-updater.exe"));
    // The stage only held the download; it is not needed any more.
    let _ = std::fs::remove_dir_all(&staged.stage_directory);
    Ok(())
}

#[cfg(target_os = "windows")]
fn file_sha256(path: &Path) -> Result<String, String> {
    let mut input = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count =
            std::io::Read::read(&mut input, &mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// The rename swap itself: `files` are (the new file, the path it replaces in `directory`).
#[cfg(target_os = "windows")]
fn install_files(files: &[(PathBuf, PathBuf)], directory: &Path) -> Result<(), String> {
    // Older backups and copies from an interrupted attempt go first (the running version's backup may be locked).
    remove_previous_backups(directory);
    // Copy and verify everything first, under names nothing else uses.
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut incoming = Vec::new();
    for (source, target) in files {
        let copy = directory.join(format!(".{}.incoming.{id}.tmp", file_stem(target)));
        let result = std::fs::copy(source, &copy)
            .map_err(|error| error.to_string())
            .and_then(|_| {
                if file_sha256(&copy)? == file_sha256(source)? {
                    Ok(())
                } else {
                    Err("更新檔案複製後驗證失敗。".to_owned())
                }
            });
        if let Err(error) = result {
            let _ = std::fs::remove_file(&copy);
            for (path, _) in &incoming {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
        incoming.push((copy, target.clone()));
    }
    // Rename the old files away, then the new ones into place.
    let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut swap = || -> Result<(), String> {
        for (copy, target) in &incoming {
            if target.exists() {
                let backup = directory.join(format!(".{}.previous.{id}.bak", file_stem(target)));
                std::fs::rename(target, &backup)
                    .map_err(|error| format!("無法將目前的 {} 改名：{error}", file_stem(target)))?;
                backups.push((target.clone(), backup));
            }
            std::fs::rename(copy, target).map_err(|error| error.to_string())?;
        }
        Ok(())
    };
    if let Err(error) = swap() {
        // Put the original names back (a target that already got its new file is removed first).
        for (target, backup) in backups.iter().rev() {
            let _ = std::fs::remove_file(target);
            let _ = std::fs::rename(backup, target);
        }
        for (copy, _) in &incoming {
            let _ = std::fs::remove_file(copy);
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Older backups of the previous versions (the one currently running may still be locked; it stays until next time).
#[cfg(target_os = "windows")]
fn remove_previous_backups(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let backup = (name.starts_with(".moemusicplayer.previous.")
            || name.starts_with(".moemusicplayer-updater.previous."))
            && name.ends_with(".bak");
        let leftover = name.contains(".incoming.")
            && name.ends_with(".tmp")
            && name.starts_with(".moemusicplayer");
        if backup || leftover {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Starts the freshly installed executable; it waits for this process to exit before it starts up
/// (`--update-relaunch <pid>`, see `wait_for_relaunch_parent`), so the single-instance check does not see us.
#[cfg(target_os = "windows")]
pub(crate) fn spawn_relaunch(target: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(target)
        .arg("--update-relaunch")
        .arg(std::process::id().to_string())
        .current_dir(target.parent().unwrap_or_else(|| Path::new(".")))
        .creation_flags(0x0000_0008 | 0x0000_0200)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("無法啟動新版：{error}"))
}

/// First thing in `main` for `--update-relaunch <pid>`: wait (at most a minute) for the old process to exit.
#[cfg(target_os = "windows")]
pub fn wait_for_relaunch_parent(pid: &str) {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    }
    let Ok(pid) = pid.parse::<u32>() else {
        return;
    };
    // SYNCHRONIZE: wait on the process object; a process that is gone cannot be opened and needs no wait.
    unsafe {
        let handle = OpenProcess(0x0010_0000, 0, pid);
        if !handle.is_null() {
            WaitForSingleObject(handle, 60_000);
            CloseHandle(handle);
        }
    }
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
    #[tokio::test(flavor = "current_thread")]
    async fn download_watchdog_bounds_stalls_and_accepts_progress() {
        assert_eq!(
            wait_for_download_progress(std::future::ready(42), Duration::from_millis(10))
                .await
                .unwrap(),
            42
        );
        let error =
            wait_for_download_progress(std::future::pending::<()>(), Duration::from_millis(5))
                .await
                .unwrap_err();
        assert!(error.contains("沒有收到資料"));
        assert_eq!(DOWNLOAD_TOTAL_TIMEOUT, Duration::from_secs(1800));
        assert_eq!(DOWNLOAD_IDLE_TIMEOUT, Duration::from_secs(30));
    }
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

#[cfg(test)]
mod version_tests {
    use super::*;

    fn asset(name: &str, digest: Option<&str>) -> Asset {
        Asset {
            name: name.into(),
            size: 1234,
            digest: digest.map(str::to_owned),
            browser_download_url: format!(
                "https://github.com/XPRAMT/MoeMusicPlayer/releases/download/v26.10.19/{name}"
            ),
        }
    }

    fn release(tag: &str, assets: Vec<Asset>) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: format!("{REPOSITORY}/releases/tag/{tag}"),
            published_at: "2026-10-19T00:00:00Z".into(),
            body: Some("notes".into()),
            assets,
        }
    }

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn versions_are_dates_compared_as_numbers() {
        assert_eq!(parse_version("v26.10.19"), Some((26, 10, 19)));
        assert_eq!(parse_version("26.10.9"), Some((26, 10, 9)));
        for bad in [
            "",
            "v26.10",
            "26.10.19.1",
            "26.10.x",
            "v26..19",
            "latest",
            "26.10.19-1",
        ] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(
            is_newer_version("v26.10.19", "26.10.9"),
            "19 is newer than 9, not older as text"
        );
        assert!(is_newer_version("27.1.1", "26.12.31"));
        assert!(
            !is_newer_version("v26.10.19", "26.10.19"),
            "the same day is not an update"
        );
        assert!(!is_newer_version("v26.10.9", "26.10.19"));
        assert!(!is_newer_version("garbage", "26.10.19"));
    }

    #[test]
    fn an_ignored_version_is_offered_again_only_by_hand() {
        assert_eq!(check_status(true, false, false), "available");
        assert_eq!(check_status(true, true, false), "ignored", "startup check: no prompt for an ignored version");
        assert_eq!(check_status(true, true, true), "available", "a manual check offers it again");
        assert_eq!(check_status(false, true, false), "current");
        assert_eq!(check_status(false, false, true), "current");
    }

    #[test]
    fn the_platform_asset_needs_a_version_tag_and_a_digest() {
        let ok = available_release(
            release(
                "v26.10.19",
                vec![
                    asset(WINDOWS_ZIP_NAME, Some(DIGEST)),
                    asset(ANDROID_APK_NAME, Some(DIGEST)),
                ],
            ),
            WINDOWS_ZIP_NAME,
        )
        .unwrap()
        .unwrap();
        assert_eq!(ok.version, "26.10.19");
        assert_eq!(ok.identity.size, 1234);
        assert_eq!(ok.identity.sha256.len(), 64);
        // A Release without this platform's file is simply not published for it.
        assert!(available_release(
            release("v26.10.19", vec![asset(WINDOWS_ZIP_NAME, Some(DIGEST))]),
            ANDROID_APK_NAME
        )
        .unwrap()
        .is_none());
        // Not a version tag, no digest, duplicated asset: refused.
        assert!(available_release(
            release(
                "v26.10.19-20261019T042347Z",
                vec![asset(WINDOWS_ZIP_NAME, Some(DIGEST))]
            ),
            WINDOWS_ZIP_NAME
        )
        .is_err());
        assert!(available_release(
            release("v26.10.19", vec![asset(WINDOWS_ZIP_NAME, None)]),
            WINDOWS_ZIP_NAME
        )
        .is_err());
        assert!(available_release(
            release(
                "v26.10.19",
                vec![
                    asset(WINDOWS_ZIP_NAME, Some(DIGEST)),
                    asset(WINDOWS_ZIP_NAME, Some(DIGEST))
                ]
            ),
            WINDOWS_ZIP_NAME
        )
        .is_err());
    }
}

#[cfg(all(test, target_os = "windows"))]
mod install_tests {
    use super::*;

    /// A scratch directory under the system temp directory, removed on drop.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moe-install-test-{}",
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn backups(directory: &Path, prefix: &str) -> Vec<PathBuf> {
        std::fs::read_dir(directory)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(prefix)
            })
            .collect()
    }

    #[test]
    fn rename_swap_installs_the_new_file_and_keeps_one_backup() {
        let root = TempDir::new();
        let directory = root.0.join("app");
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("moemusicplayer.exe");
        std::fs::write(&target, b"old exe").unwrap();
        let source = root.0.join("new.exe");
        std::fs::write(&source, b"new exe").unwrap();
        install_files(&[(source.clone(), target.clone())], &directory).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new exe");
        let kept = backups(&directory, ".moemusicplayer.previous.");
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read(&kept[0]).unwrap(), b"old exe");
        assert!(
            backups(&directory, ".moemusicplayer.incoming.").is_empty(),
            "no copy is left behind"
        );
        // A second update drops the older backup and keeps only the previous version.
        std::fs::write(&source, b"newer exe").unwrap();
        install_files(&[(source, target.clone())], &directory).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"newer exe");
        let kept = backups(&directory, ".moemusicplayer.previous.");
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read(&kept[0]).unwrap(), b"new exe");
    }

    #[test]
    fn a_missing_source_replaces_nothing() {
        let root = TempDir::new();
        let directory = root.0.join("app");
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("moemusicplayer.exe");
        std::fs::write(&target, b"old exe").unwrap();
        assert!(
            install_files(&[(root.0.join("missing.exe"), target.clone())], &directory).is_err()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"old exe");
        assert!(
            backups(&directory, ".").is_empty(),
            "no temporary copy or backup remains"
        );
    }

    #[test]
    fn the_executable_comes_out_of_the_release_zip() {
        use std::io::Write;
        let root = TempDir::new();
        let archive = root.0.join("release.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("moemusicplayer.exe", options).unwrap();
            zip.write_all(b"MZ pretend executable").unwrap();
            zip.start_file("readme.txt", options).unwrap();
            zip.write_all(b"ignored").unwrap();
            zip.finish().unwrap();
        }
        let output = root.0.join("out.exe");
        extract_executable(&archive, &output).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"MZ pretend executable");
        // A zip without the executable is refused.
        let other = root.0.join("other.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&other).unwrap());
            zip.start_file("readme.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        assert!(extract_executable(&other, &root.0.join("never.exe")).is_err());
    }

    #[test]
    fn a_running_image_can_be_renamed_away() {
        // The point of the rename swap: Windows lets a running executable be renamed even though it cannot be
        // overwritten or deleted. Use a copy of the test binary as the "running" file.
        let current = std::env::current_exe().unwrap();
        let directory = TempDir::new();
        let copy = directory.0.join("running.exe");
        std::fs::copy(&current, &copy).unwrap();
        let mut child = std::process::Command::new(&copy)
            .args([
                "--exact",
                "updater::install_tests::sleep_helper",
                "--ignored",
                "--nocapture",
            ])
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            std::fs::remove_file(&copy).is_err(),
            "a running image cannot be deleted"
        );
        let moved = directory.0.join("running.exe.bak");
        std::fs::rename(&copy, &moved).expect("a running image can be renamed");
        std::fs::write(&copy, b"new").unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    #[ignore = "helper process for a_running_image_can_be_renamed_away"]
    fn sleep_helper() {
        std::thread::sleep(Duration::from_secs(5));
    }
}
