use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use walkdir::WalkDir;

use crate::settings::SlideshowOrder;

const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct MainBackgroundRuntime {
    slideshow: Mutex<Option<Slideshow>>,
}

struct Slideshow {
    folder: PathBuf,
    files: Vec<PathBuf>,
    order: SlideshowOrder,
    indices: Vec<usize>,
    cursor: usize,
    last_index: Option<usize>,
    random_state: u64,
}

impl MainBackgroundRuntime {
    pub(crate) fn configure_slideshow(
        &self,
        folder: &Path,
        order: SlideshowOrder,
    ) -> Result<(), String> {
        self.configure_slideshow_with(folder, order, scan_slideshow_images)
    }

    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    /// Same as configure_slideshow with a caller-provided scanner (Android SAF trees list content URIs,
    /// carried as lossless UTF-8 PathBufs).
    pub(crate) fn configure_slideshow_with(
        &self,
        folder: &Path,
        order: SlideshowOrder,
        scan: impl FnOnce(&Path) -> Result<Vec<PathBuf>, String>,
    ) -> Result<(), String> {
        let files = scan(folder)?;
        if files.is_empty() {
            return Err("所選資料夾及其子資料夾中沒有可用的圖片。".to_owned());
        }
        let mut state = self
            .slideshow
            .lock()
            .map_err(|_| "幻燈片背景狀態不可用。".to_owned())?;
        *state = Some(Slideshow::new(folder.to_path_buf(), files, order));
        Ok(())
    }

    pub(crate) fn next_slideshow_image(
        &self,
        folder: &Path,
        order: SlideshowOrder,
    ) -> Result<PathBuf, String> {
        self.next_slideshow_image_with(folder, order, scan_slideshow_images)
    }

    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(crate) fn next_slideshow_image_with(
        &self,
        folder: &Path,
        order: SlideshowOrder,
        scan: impl FnOnce(&Path) -> Result<Vec<PathBuf>, String>,
    ) -> Result<PathBuf, String> {
        let mut state = self
            .slideshow
            .lock()
            .map_err(|_| "幻燈片背景狀態不可用。".to_owned())?;
        let needs_refresh = state
            .as_ref()
            .is_none_or(|current| current.folder != folder || current.order != order);
        if needs_refresh {
            let files = scan(folder)?;
            if files.is_empty() {
                *state = None;
                return Err("所選資料夾及其子資料夾中沒有可用的圖片。".to_owned());
            }
            *state = Some(Slideshow::new(folder.to_path_buf(), files, order));
        }
        state
            .as_mut()
            .and_then(Slideshow::next)
            .ok_or_else(|| "幻燈片資料夾中沒有可用的圖片。".to_owned())
    }
}

impl Slideshow {
    fn new(folder: PathBuf, files: Vec<PathBuf>, order: SlideshowOrder) -> Self {
        let mut slideshow = Self {
            folder,
            indices: (0..files.len()).collect(),
            files,
            order,
            cursor: 0,
            last_index: None,
            random_state: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(0x9e37_79b9_7f4a_7c15),
        };
        if order == SlideshowOrder::Random {
            slideshow.shuffle_indices();
        }
        slideshow
    }

    fn next(&mut self) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        if self.cursor >= self.indices.len() {
            self.cursor = 0;
            if self.order == SlideshowOrder::Random {
                self.shuffle_indices();
                if self.indices.len() > 1 && self.indices.first().copied() == self.last_index {
                    self.indices.swap(0, 1);
                }
            }
        }
        let index = *self.indices.get(self.cursor)?;
        self.cursor += 1;
        self.last_index = Some(index);
        self.files.get(index).cloned()
    }

    fn shuffle_indices(&mut self) {
        for end in (1..self.indices.len()).rev() {
            self.random_state ^= self.random_state << 13;
            self.random_state ^= self.random_state >> 7;
            self.random_state ^= self.random_state << 17;
            let selected = (self.random_state as usize) % (end + 1);
            self.indices.swap(end, selected);
        }
    }
}

pub(crate) fn scan_slideshow_images(folder: &Path) -> Result<Vec<PathBuf>, String> {
    if !folder.is_dir() {
        return Err("幻燈片資料夾不存在或無法讀取。".to_owned());
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() || !is_supported_image(entry.path()) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if (1..=MAX_IMAGE_BYTES).contains(&metadata.len()) {
            files.push(entry.into_path());
        }
    }
    files.sort_by(|left, right| {
        let left = left.strip_prefix(folder).unwrap_or(left);
        let right = right.strip_prefix(folder).unwrap_or(right);
        let left_name = left.to_string_lossy();
        let right_name = right.to_string_lossy();
        left_name
            .to_lowercase()
            .cmp(&right_name.to_lowercase())
            .then_with(|| left_name.cmp(&right_name))
    });
    Ok(files)
}

pub(crate) fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
            )
        })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{scan_slideshow_images, MainBackgroundRuntime, Slideshow};
    use crate::settings::SlideshowOrder;

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("moe-slideshow-{nonce}"));
        fs::create_dir_all(&path).expect("create directory");
        path
    }

    #[test]
    fn slideshow_scan_includes_nested_supported_images_in_case_insensitive_order() {
        let root = temp_dir();
        fs::create_dir_all(root.join("sub")).expect("nested directory");
        fs::write(root.join("b.JPG"), [0xff, 0xd8, 0xff]).expect("jpeg");
        fs::write(root.join("sub").join("a.png"), b"png").expect("png");
        fs::write(root.join("ignore.txt"), b"text").expect("text");
        let files = scan_slideshow_images(&root).expect("scan images");
        let relative = files
            .iter()
            .map(|path| path.strip_prefix(&root).expect("relative").to_path_buf())
            .collect::<Vec<_>>();
        assert_eq!(
            relative,
            vec![PathBuf::from("b.JPG"), PathBuf::from("sub").join("a.png")]
        );
        fs::remove_dir_all(root).expect("remove directory");
    }

    #[test]
    fn injected_scanner_drives_uri_keyed_slideshow_and_rescans_only_on_source_change() {
        let runtime = MainBackgroundRuntime::default();
        let tree = PathBuf::from("content://com.android.externalstorage.documents/tree/primary%3APictures");
        let uris = vec![
            PathBuf::from("content://docs/tree/primary%3APictures/document/primary%3APictures%2Fa.png"),
            PathBuf::from("content://docs/tree/primary%3APictures/document/primary%3APictures%2Fb.jpg"),
        ];
        let scans = std::cell::Cell::new(0);
        let scan = |_: &std::path::Path| {
            scans.set(scans.get() + 1);
            Ok(uris.clone())
        };
        runtime
            .configure_slideshow_with(&tree, SlideshowOrder::Sequential, scan)
            .expect("configure");
        let first = runtime
            .next_slideshow_image_with(&tree, SlideshowOrder::Sequential, scan)
            .expect("first");
        let second = runtime
            .next_slideshow_image_with(&tree, SlideshowOrder::Sequential, scan)
            .expect("second");
        assert_eq!(vec![first, second], uris);
        assert_eq!(scans.get(), 1);
        let empty = runtime.next_slideshow_image_with(
            &PathBuf::from("content://other"),
            SlideshowOrder::Sequential,
            |_| Ok(Vec::new()),
        );
        assert!(empty.is_err());
    }

    #[test]
    fn random_slideshow_visits_each_image_once_per_cycle_and_avoids_same_cycle_boundary() {
        let files = (0..8)
            .map(|index| PathBuf::from(format!("{index}.jpg")))
            .collect::<Vec<_>>();
        let mut slideshow = Slideshow::new(
            PathBuf::from("slides"),
            files.clone(),
            SlideshowOrder::Random,
        );
        slideshow.random_state = 0x1_2345_6789;
        slideshow.shuffle_indices();
        let first_cycle = (0..files.len())
            .map(|_| slideshow.next().expect("image"))
            .collect::<Vec<_>>();
        let unique = first_cycle.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), files.len());
        let previous = first_cycle.last().expect("last item").to_path_buf();
        let next_cycle_first = slideshow.next().expect("next cycle image");
        assert_ne!(next_cycle_first, previous);
    }
}
