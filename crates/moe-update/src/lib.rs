//! Narrow portable updater protocol and Windows process/file operations.
//! No network, arbitrary command, registry, service, or reboot capabilities.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub version: String,
    pub build_timestamp_utc: String,
    pub git_commit: String,
    pub architecture: String,
    pub executable: FileIdentity,
    pub helper: FileIdentity,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileIdentity {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingUpdate {
    pub manifest: ReleaseManifest,
    pub stage_directory: PathBuf,
    pub target: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyPlan {
    pub pending: PendingUpdate,
    pub parent_pid: u32,
    pub parent_started: u64,
    pub original_sha256: String,
    pub nonce: String,
    pub wait_seconds: u32,
}

pub fn file_sha256(path: &Path) -> Result<String, String> {
    let mut input = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn verify_file(path: &Path, identity: &FileIdentity) -> Result<(), String> {
    if std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .len()
        != identity.size
        || file_sha256(path)? != identity.sha256.to_ascii_lowercase()
    {
        return Err(format!("更新檔案驗證失敗：{}", identity.name));
    }
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let input = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    input
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 64 * 1024 {
        return Err("更新紀錄超過大小上限。".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(&serde_json::to_vec(value).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

pub fn validate_pending(pending: &PendingUpdate, actual_exe: &Path) -> Result<(), String> {
    let target = pending
        .target
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if target
        != actual_exe
            .canonicalize()
            .map_err(|error| error.to_string())?
        || target.file_name().and_then(|name| name.to_str()) != Some("moemusicplayer.exe")
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
    let stage = pending
        .stage_directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if stage.parent() != Some(updates.as_path()) {
        return Err("更新暫存必須直接位於 UserData/updates。".into());
    }
    if pending.manifest.schema_version != 1
        || pending.manifest.architecture != "windows-x86_64"
        || pending.manifest.executable.name != "moemusicplayer.exe"
        || pending.manifest.helper.name != "moemusicplayer-updater.exe"
    {
        return Err("更新清單版本、架構或檔名無效。".into());
    }
    for identity in [&pending.manifest.executable, &pending.manifest.helper] {
        let file = stage.join(&identity.name);
        if file
            .canonicalize()
            .map_err(|error| error.to_string())?
            .parent()
            != Some(stage.as_path())
        {
            return Err("更新檔案不可使用外部連結。".into());
        }
        verify_file(&file, identity)?;
    }
    Ok(())
}

#[cfg(windows)]
pub mod windows;
