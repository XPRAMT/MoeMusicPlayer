//! A restored `settings.json` may point at a background that this device does not have (a backup does not carry the
//! image): the single image lives in `UserData/background` and the slideshow folder is a path in the settings. A missing
//! image or folder is not an error, the main background just stays plain.

use std::path::{Path, PathBuf};

/// The single main background image in the user-data folder, when it exists.
pub(crate) fn single_image_path(user_data: &Path, file_name: &str) -> Option<PathBuf> {
    let path = user_data.join(file_name);
    path.is_file().then_some(path)
}

/// Whether the slideshow folder is there; when it is not, the background stays plain without an error.
pub(crate) fn slideshow_folder_exists(folder: &Path) -> bool {
    folder.is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AppSettings, MainBackgroundMode, SettingsStore, StoredPath};

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("moe-bg-fallback-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// A settings file as a backup carries it, reopened like the next start does.
    fn restored(dir: &Path, mode: MainBackgroundMode, folder: Option<&Path>) -> crate::settings::AppSettings {
        let file = dir.join("settings.json");
        let store = SettingsStore::open(file.clone(), AppSettings::default()).unwrap();
        store
            .update(|settings| {
                settings.theme.main_background_mode = mode;
                settings.theme.slideshow_folder = folder.map(|folder| StoredPath::from_path(folder).unwrap());
                Ok(())
            })
            .unwrap();
        drop(store);
        SettingsStore::open(file, AppSettings::default()).unwrap().snapshot().unwrap()
    }

    #[test]
    fn a_restored_single_image_setting_without_the_image_falls_back_quietly() {
        let dir = scratch("single");
        let settings = restored(&dir, MainBackgroundMode::SingleImage, None);
        assert_eq!(settings.theme.main_background_mode, MainBackgroundMode::SingleImage, "the setting is kept");
        assert!(single_image_path(&dir, "background").is_none(), "no image: nothing to read, no error");
        std::fs::write(dir.join("background"), b"image").unwrap();
        assert_eq!(single_image_path(&dir, "background"), Some(dir.join("background")), "the image is back in use");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_restored_slideshow_folder_that_is_gone_falls_back_quietly_and_one_that_exists_is_kept() {
        let dir = scratch("slideshow");
        let missing = dir.join("pictures-that-are-gone");
        let settings = restored(&dir, MainBackgroundMode::Slideshow, Some(&missing));
        assert_eq!(settings.theme.main_background_mode, MainBackgroundMode::Slideshow);
        let stored = settings.theme.slideshow_folder.expect("the path stays in the settings").to_path_buf().unwrap();
        assert_eq!(stored, missing);
        assert!(!slideshow_folder_exists(&stored));

        let present = dir.join("pictures");
        std::fs::create_dir_all(&present).unwrap();
        let settings = restored(&dir, MainBackgroundMode::Slideshow, Some(&present));
        let stored = settings.theme.slideshow_folder.unwrap().to_path_buf().unwrap();
        assert_eq!(stored, present);
        assert!(slideshow_folder_exists(&stored));
        let _ = std::fs::remove_dir_all(dir);
    }
}
