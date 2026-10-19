//! Portable Windows update metadata and staging, and the Android APK update (ADR 0015). All
//! locations are owned by this service; the renderer can select timing, never a URL or path.
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
const DOWNLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Android assets in the same GitHub Release as the Windows pair (ADR 0015).
const ANDROID_MANIFEST_NAME: &str = "moemusicplayer-android-update.json";
const ANDROID_APK_NAME: &str = "moemusicplayer-android-arm64.apk";
const ANDROID_ARCHITECTURE: &str = "android-arm64";

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

/// `moemusicplayer-android-update.json`: same build identity fields as the Windows manifest, one
/// APK. A separate asset keeps older Windows clients (strict `deny_unknown_fields`) unaffected.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AndroidReleaseManifest {
    pub schema_version: u32,
    pub version: String,
    pub build_timestamp_utc: String,
    pub git_commit: String,
    pub architecture: String,
    pub apk: FileIdentity,
}

#[derive(Clone)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
struct AvailableAndroidRelease {
    manifest: AndroidReleaseManifest,
    apk: Asset,
}

#[derive(Clone)]
#[cfg_attr(target_os = "android", allow(dead_code))]
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
    android_available: Mutex<Option<AvailableAndroidRelease>>,
    /// Verified APK already downloaded for `android_available` (retry after the install prompt).
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
            android_available: Mutex::new(None),
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
        .timeout(Duration::from_secs(15))
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

fn validate_android_manifest(manifest: &AndroidReleaseManifest) -> Result<(), String> {
    if manifest.schema_version != 1 || manifest.architecture != ANDROID_ARCHITECTURE {
        return Err("此 Android 更新版本或架構不受支援。".into());
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
    let apk = &manifest.apk;
    if apk.name != ANDROID_APK_NAME
        || !valid_hash(&apk.sha256)
        || apk.size == 0
        || apk.size > MAX_DOWNLOAD
    {
        return Err("Android 更新檔案清單無效。".into());
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

/// Download a manifest asset and verify it against GitHub's SHA-256 asset digest.
async fn verified_manifest_bytes(
    client: &reqwest::Client,
    asset: &Asset,
) -> Result<Vec<u8>, String> {
    if !trusted_download_url(&asset.browser_download_url) {
        return Err("更新清單來源無效。".into());
    }
    let hash = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| valid_hash(hash))
        .ok_or("GitHub 未提供更新清單 SHA-256 digest；為保護完整性，無法自動更新。")?;
    let bytes = bounded_bytes(client, &asset.browser_download_url, 64 * 1024).await?;
    if bytes.len() as u64 != asset.size || sha256(&bytes) != hash.to_ascii_lowercase() {
        return Err("更新清單 SHA-256 驗證失敗。".into());
    }
    Ok(bytes)
}

async fn check_release() -> Result<Option<AvailableRelease>, String> {
    let client = client()?;
    let Some(release) = latest_release(&client).await? else {
        return Ok(None);
    };
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == MANIFEST_NAME)
        .ok_or("Release 缺少可驗證的更新清單。")?;
    let bytes = verified_manifest_bytes(&client, asset).await?;
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

/// Android: the latest Release's APK manifest. A Release without Android assets is "unpublished"
/// for Android (Windows-only releases stay valid).
async fn check_android_release() -> Result<Option<(AvailableAndroidRelease, String, String)>, String>
{
    let client = client()?;
    let Some(release) = latest_release(&client).await? else {
        return Ok(None);
    };
    let Some(asset) = release
        .assets
        .iter()
        .find(|asset| asset.name == ANDROID_MANIFEST_NAME)
    else {
        return Ok(None);
    };
    let bytes = verified_manifest_bytes(&client, asset).await?;
    let manifest: AndroidReleaseManifest =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    validate_android_manifest(&manifest)?;
    let apk = matching_asset(&release, &manifest.apk)?;
    Ok(Some((
        AvailableAndroidRelease { manifest, apk },
        release.published_at,
        release.body.unwrap_or_default(),
    )))
}

async fn update_check_android(app: &AppHandle, service: &UpdateService) -> UpdateState {
    publish(app, service, |state| {
        state.status = "checking".into();
        state.error = None;
    });
    *service
        .android_available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    match check_android_release().await {
        Ok(None) => publish(app, service, |state| {
            state.status = "unpublished".into();
            state.latest = None;
        }),
        Ok(Some((release, published_at, notes))) => {
            let is_new =
                release.manifest.build_timestamp_utc.as_str() > env!("MOE_BUILD_TIMESTAMP_UTC");
            let state = publish(app, service, |state| {
                state.status = if is_new { "available" } else { "current" }.into();
                state.latest = Some(LatestBuild {
                    build_id: format!(
                        "{}@{}",
                        release.manifest.build_timestamp_utc, release.manifest.git_commit
                    ),
                    version: release.manifest.version.clone(),
                    published_at,
                    notes,
                    download_bytes: release.manifest.apk.size,
                });
            });
            *service
                .android_available
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(release);
            state
        }
        Err(error) => failed(app, service, error),
    }
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
    if cfg!(all(target_os = "android", target_arch = "aarch64")) {
        return Ok(update_check_android(&app, &service).await);
    }
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Ok(publish(&app, &service, |state| {
            state.status = "unsupported".into();
            state.error = Some("自動更新目前僅支援 Windows x64 便攜版與 Android arm64。".into());
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

#[cfg(target_os = "windows")]
async fn stage_release(
    app: Option<&AppHandle>,
    service: &UpdateService,
    release: AvailableRelease,
) -> Result<PendingUpdate, String> {
    let directory = updates_directory()?;
    let stage_directory = directory.join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&stage_directory).map_err(|error| error.to_string())?;
    let result = tokio::time::timeout(DOWNLOAD_TOTAL_TIMEOUT, async {
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
        Ok(pending)
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
        if release.manifest.build_timestamp_utc.as_str() <= env!("MOE_BUILD_TIMESTAMP_UTC") {
            return Err("此版本不比目前建置新。".into());
        }
        publish(&app, &service, |state| {
            state.status = "downloading".into();
            state.downloaded_bytes = 0;
            state.total_bytes =
                Some(release.manifest.executable.size + release.manifest.helper.size);
            state.error = None;
        });
        #[cfg(target_os = "windows")]
        let result = async {
            let pending = stage_release(Some(&app), &service, release).await?;
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                crate::install_downloaded_update(
                    &app,
                    &pending,
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
        .android_available
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or("請先檢查更新。")?;
    if release.manifest.build_timestamp_utc.as_str() <= env!("MOE_BUILD_TIMESTAMP_UTC") {
        return Err("此版本不比目前建置新。".into());
    }
    let cached = service
        .android_apk
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .filter(|path| {
            std::fs::metadata(path)
                .is_ok_and(|metadata| metadata.len() == release.manifest.apk.size)
        });
    let path = match cached {
        Some(path) => path,
        None => {
            publish(app, service, |state| {
                state.status = "downloading".into();
                state.downloaded_bytes = 0;
                state.total_bytes = Some(release.manifest.apk.size);
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
                    &release.apk,
                    &release.manifest.apk,
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
pub(crate) fn validate_for_apply(pending: &PendingUpdate) -> Result<(), String> {
    validate_manifest(&pending.manifest)?;
    moe_update::validate_pending(
        pending,
        &std::env::current_exe().map_err(|error| error.to_string())?,
    )
}

/// Installs a staged, verified release by renaming: each running image keeps its (renamed) file and the new file takes
/// the original name, so the new version starts on the next launch. A backup of the previous file stays beside it
/// (`.moemusicplayer.previous.<id>.bak`, the same naming the release script uses); only the newest one is kept.
/// Nothing is replaced unless every file was copied and verified, and a failure puts the original names back.
#[cfg(target_os = "windows")]
pub(crate) fn install_in_place(pending: &PendingUpdate) -> Result<(), String> {
    validate_for_apply(pending)?;
    let directory = pending
        .target
        .parent()
        .ok_or("更新目標缺少目錄。")?
        .to_path_buf();
    let files = [
        (&pending.manifest.executable, pending.target.clone()),
        (
            &pending.manifest.helper,
            directory.join(&pending.manifest.helper.name),
        ),
    ];
    install_files(&pending.stage_directory, &directory, &files)?;
    // The stage only held the downloads; it is not needed any more.
    let _ = std::fs::remove_dir_all(&pending.stage_directory);
    Ok(())
}

/// The rename swap itself: `files` are (identity of the staged file in `stage`, the path it replaces in `directory`).
#[cfg(target_os = "windows")]
fn install_files(
    stage: &Path,
    directory: &Path,
    files: &[(&moe_update::FileIdentity, PathBuf)],
) -> Result<(), String> {
    // Older backups and copies from an interrupted attempt go first (the running version's backup may be locked).
    remove_previous_backups(directory);
    // Copy and verify everything first, under names nothing else uses.
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut incoming = Vec::new();
    for (identity, target) in files {
        let copy = directory.join(format!(".{}.incoming.{id}.tmp", file_stem(target)));
        let result = std::fs::copy(stage.join(&identity.name), &copy)
            .map_err(|error| error.to_string())
            .and_then(|_| moe_update::verify_file(&copy, identity));
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
    if let Ok(pid) = pid.parse::<u32>() {
        moe_update::windows::wait_for_exit(pid, Duration::from_secs(60));
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
    fn android_manifest_requires_the_fixed_apk_identity() {
        let valid = AndroidReleaseManifest {
            schema_version: 1,
            version: "2026.10.17".into(),
            build_timestamp_utc: "2026-10-17T07:00:00.000Z".into(),
            git_commit: "a".repeat(40),
            architecture: ANDROID_ARCHITECTURE.into(),
            apk: FileIdentity {
                name: ANDROID_APK_NAME.into(),
                sha256: "b".repeat(64),
                size: 1024,
            },
        };
        assert!(validate_android_manifest(&valid).is_ok());
        let mut wrong_name = valid.clone();
        wrong_name.apk.name = "other.apk".into();
        assert!(validate_android_manifest(&wrong_name).is_err());
        let mut wrong_arch = valid.clone();
        wrong_arch.architecture = "windows-x86_64".into();
        assert!(validate_android_manifest(&wrong_arch).is_err());
        let json = serde_json::to_string(&valid).unwrap();
        assert!(json.contains("\"buildTimestampUtc\"") && json.contains("\"apk\""));
        let extra = json.replacen('{', "{\"extra\":1,", 1);
        assert!(serde_json::from_str::<AndroidReleaseManifest>(&extra).is_err());
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

    fn identity(name: &str, bytes: &[u8]) -> moe_update::FileIdentity {
        moe_update::FileIdentity {
            name: name.into(),
            sha256: sha256(bytes),
            size: bytes.len() as u64,
        }
    }

    fn setup(old: &[u8]) -> (TempDir, PathBuf, PathBuf) {
        let root = TempDir::new();
        let directory = root.0.join("app");
        let stage = directory.join("UserData").join("updates").join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(directory.join("moemusicplayer.exe"), old).unwrap();
        std::fs::write(directory.join("moemusicplayer-updater.exe"), b"old helper").unwrap();
        (root, directory, stage)
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
    fn rename_swap_installs_new_files_and_keeps_one_backup() {
        let (_root, directory, stage) = setup(b"old exe");
        std::fs::write(stage.join("moemusicplayer.exe"), b"new exe").unwrap();
        std::fs::write(stage.join("moemusicplayer-updater.exe"), b"new helper").unwrap();
        let exe = identity("moemusicplayer.exe", b"new exe");
        let helper = identity("moemusicplayer-updater.exe", b"new helper");
        let files = [
            (&exe, directory.join("moemusicplayer.exe")),
            (&helper, directory.join("moemusicplayer-updater.exe")),
        ];
        install_files(&stage, &directory, &files).unwrap();
        assert_eq!(
            std::fs::read(directory.join("moemusicplayer.exe")).unwrap(),
            b"new exe"
        );
        assert_eq!(
            std::fs::read(directory.join("moemusicplayer-updater.exe")).unwrap(),
            b"new helper"
        );
        let exe_backups = backups(&directory, ".moemusicplayer.previous.");
        assert_eq!(exe_backups.len(), 1);
        assert_eq!(std::fs::read(&exe_backups[0]).unwrap(), b"old exe");
        assert!(
            backups(&directory, ".moemusicplayer.incoming.").is_empty(),
            "no copy is left behind"
        );
        // A second update drops the older backup and keeps only the previous version.
        std::fs::write(stage.join("moemusicplayer.exe"), b"newer exe").unwrap();
        std::fs::write(stage.join("moemusicplayer-updater.exe"), b"newer helper").unwrap();
        let exe2 = identity("moemusicplayer.exe", b"newer exe");
        let helper2 = identity("moemusicplayer-updater.exe", b"newer helper");
        let files = [
            (&exe2, directory.join("moemusicplayer.exe")),
            (&helper2, directory.join("moemusicplayer-updater.exe")),
        ];
        install_files(&stage, &directory, &files).unwrap();
        let exe_backups = backups(&directory, ".moemusicplayer.previous.");
        assert_eq!(exe_backups.len(), 1);
        assert_eq!(std::fs::read(&exe_backups[0]).unwrap(), b"new exe");
    }

    #[test]
    fn a_failed_verification_replaces_nothing() {
        let (_root, directory, stage) = setup(b"old exe");
        std::fs::write(stage.join("moemusicplayer.exe"), b"corrupted").unwrap();
        std::fs::write(stage.join("moemusicplayer-updater.exe"), b"new helper").unwrap();
        let exe = identity("moemusicplayer.exe", b"new exe");
        let helper = identity("moemusicplayer-updater.exe", b"new helper");
        let files = [
            (&exe, directory.join("moemusicplayer.exe")),
            (&helper, directory.join("moemusicplayer-updater.exe")),
        ];
        assert!(install_files(&stage, &directory, &files).is_err());
        assert_eq!(
            std::fs::read(directory.join("moemusicplayer.exe")).unwrap(),
            b"old exe"
        );
        assert_eq!(
            std::fs::read(directory.join("moemusicplayer-updater.exe")).unwrap(),
            b"old helper"
        );
        assert!(
            backups(&directory, ".").is_empty(),
            "no temporary copy or backup remains"
        );
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
