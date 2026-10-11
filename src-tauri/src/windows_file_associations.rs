use std::path::Path;

use player_platform_windows::supported_audio_extensions;
use windows_registry::CURRENT_USER;

const CAPABILITIES_PATH: &str = r"Software\MoeMusicPlayer\Capabilities";
const REGISTERED_APPS_PATH: &str = r"Software\RegisteredApplications";
const APPLICATION_NAME: &str = "MoeMusicPlayer";
pub fn register_file_associations(executable: &Path) -> Result<(), String> {
    let executable = executable
        .canonicalize()
        .map_err(|error| format!("無法解析播放器執行檔路徑：{error}"))?;
    let executable = executable.to_string_lossy();
    let capabilities = CURRENT_USER
        .create(CAPABILITIES_PATH)
        .map_err(|error| format!("無法建立播放器格式登錄資料：{error}"))?;
    capabilities
        .set_string("ApplicationName", APPLICATION_NAME)
        .map_err(|error| error.to_string())?;
    capabilities
        .set_string("ApplicationDescription", "MoeMusicPlayer 音樂播放器")
        .map_err(|error| error.to_string())?;
    capabilities
        .set_string("ApplicationIcon", format!("{executable},0"))
        .map_err(|error| error.to_string())?;

    let file_associations = CURRENT_USER
        .create(format!(r"{CAPABILITIES_PATH}\FileAssociations"))
        .map_err(|error| format!("無法建立音訊格式清單：{error}"))?;
    for extension in supported_audio_extensions() {
        let extension = extension.as_str();
        let prog_id = prog_id(extension);
        file_associations
            .set_string(extension, &prog_id)
            .map_err(|error| format!("無法登錄 {extension}：{error}"))?;

        let class_key = CURRENT_USER
            .create(format!(r"Software\Classes\{prog_id}\shell\open\command"))
            .map_err(|error| format!("無法建立 {extension} 開啟動作：{error}"))?;
        class_key
            .set_string("", open_command(&executable))
            .map_err(|error| format!("無法登錄 {extension} 播放器路徑：{error}"))?;

        let display_key = CURRENT_USER
            .create(format!(r"Software\Classes\{prog_id}"))
            .map_err(|error| format!("無法建立 {extension} 顯示名稱：{error}"))?;
        display_key
            .set_string("", format!("MoeMusicPlayer {extension} 音訊"))
            .map_err(|error| error.to_string())?;

        CURRENT_USER
            .create(format!(r"Software\Classes\{extension}\OpenWithProgids"))
            .and_then(|key| key.set_string(&prog_id, ""))
            .map_err(|error| format!("無法將播放器加入 {extension} 的開啟方式：{error}"))?;
    }

    CURRENT_USER
        .create(REGISTERED_APPS_PATH)
        .and_then(|key| key.set_string(APPLICATION_NAME, CAPABILITIES_PATH))
        .map_err(|error| format!("無法登錄 Windows 預設應用程式清單：{error}"))?;
    Ok(())
}

pub fn open_default_apps_settings() -> Result<(), String> {
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

    let uri: Vec<u16> = std::ffi::OsStr::new(default_apps_settings_uri())
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            uri.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if result <= 32 {
        return Err(format!("無法開啟 Windows 預設應用程式設定（{result}）。"));
    }
    Ok(())
}

fn default_apps_settings_uri() -> &'static str {
    "ms-settings:defaultapps?registeredAppUser=MoeMusicPlayer"
}

fn prog_id(extension: &str) -> String {
    format!("MoeMusicPlayer.Audio{}", extension.trim_start_matches('.'))
}

fn open_command(executable: &str) -> String {
    format!("\"{executable}\" \"%1\"")
}

#[cfg(test)]
mod tests {
    use super::{default_apps_settings_uri, open_command, prog_id};

    #[test]
    fn associations_cover_supported_audio_without_video_mp4() {
        let extensions = super::supported_audio_extensions();
        assert!(extensions.contains(&".m4a".to_owned()));
        assert!(extensions.contains(&".mp3".to_owned()));
        assert!(!extensions.contains(&".mp4".to_owned()));
    }

    #[test]
    fn prog_ids_are_stable_and_commands_quote_paths_and_file_arguments() {
        assert_eq!(prog_id(".m4a"), "MoeMusicPlayer.Audiom4a");
        assert_eq!(
            open_command(r"C:\Program Files\MoeMusicPlayer\moemusicplayer.exe"),
            r#""C:\Program Files\MoeMusicPlayer\moemusicplayer.exe" "%1""#
        );
    }

    #[test]
    fn default_apps_settings_is_launched_through_the_windows_uri_scheme() {
        assert_eq!(
            default_apps_settings_uri(),
            "ms-settings:defaultapps?registeredAppUser=MoeMusicPlayer"
        );
    }
}
