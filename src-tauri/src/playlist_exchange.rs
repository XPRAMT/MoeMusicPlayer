use std::{fs, path::Path};

use player_core::{
    parse_m3u, parse_m3u8, write_m3u, write_m3u8, M3uExportOptions, PlaylistId, PlaylistSummary,
};
use player_db::Database;
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistImportResult {
    pub playlist: PlaylistSummary,
    pub matched_entries: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistExportResult {
    pub playlist_name: String,
    pub entry_count: u64,
}

pub fn import_playlist_file(
    database: &Database,
    path: &Path,
) -> Result<PlaylistImportResult, String> {
    let bytes = fs::read(path).map_err(|error| format!("無法讀取選擇的播放清單：{error}"))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or_else(|| "播放清單必須使用 .m3u 或 .m3u8 副檔名。".to_owned())?;
    let mut playlist = if extension.eq_ignore_ascii_case("m3u8") {
        parse_m3u8(&bytes, path)
    } else if extension.eq_ignore_ascii_case("m3u") {
        parse_m3u(&bytes, path)
    } else {
        return Err("只支援匯入 .m3u 或 .m3u8 播放清單。".to_owned());
    }
    .map_err(|error| format!("播放清單格式無效：{error}"))?;

    if playlist.name.trim().is_empty() {
        playlist.name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("未命名播放清單")
            .to_owned();
    }

    let playlist_id = playlist.id;
    database
        .save_playlist(&playlist)
        .map_err(|error| format!("無法保存播放清單：{error}"))?;
    let saved = database
        .get_playlist(playlist_id)
        .map_err(|error| format!("無法確認匯入結果：{error}"))?
        .ok_or_else(|| "播放清單保存後無法重新讀取。".to_owned())?;
    let matched_entries = saved
        .entries
        .iter()
        .filter(|entry| entry.track_id.is_some())
        .count();
    let summary = PlaylistSummary {
        id: saved.id,
        name: saved.name,
        entry_count: u64::try_from(saved.entries.len()).unwrap_or(u64::MAX),
    };

    Ok(PlaylistImportResult {
        playlist: summary,
        matched_entries: u64::try_from(matched_entries).unwrap_or(u64::MAX),
    })
}

pub fn export_playlist_file(
    database: &Database,
    playlist_id: &str,
    path: &Path,
    format: &str,
    relative_root: Option<&Path>,
) -> Result<PlaylistExportResult, String> {
    let playlist_id = PlaylistId::parse(playlist_id)
        .map_err(|_| "播放清單識別碼無效，請重新載入清單。".to_owned())?;
    let playlist = database
        .get_playlist(playlist_id)
        .map_err(|error| format!("無法讀取播放清單：{error}"))?
        .ok_or_else(|| "找不到這份播放清單，請重新載入清單。".to_owned())?;
    let format = match format {
        "m3u" => "m3u",
        "m3u8" => "m3u8",
        _ => return Err("匯出格式必須是 M3U 或 M3U8。".to_owned()),
    };
    let output_path = if path.extension().is_none() {
        path.with_extension(format)
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(format))
    {
        path.to_path_buf()
    } else {
        return Err(format!("{format} 匯出檔必須使用 .{format} 副檔名。"));
    };
    let options = M3uExportOptions {
        relative_root: relative_root.map(Path::to_path_buf),
    };
    let bytes = match format {
        "m3u" => write_m3u(&playlist, &output_path, &options),
        "m3u8" => write_m3u8(&playlist, &output_path, &options),
        _ => unreachable!("validated above"),
    }
    .map_err(|error| format!("無法產生 UTF-8 {format}: {error}"))?;
    fs::write(&output_path, bytes).map_err(|error| format!("無法寫入 M3U8 檔案：{error}"))?;

    Ok(PlaylistExportResult {
        playlist_name: playlist.name,
        entry_count: u64::try_from(playlist.entries.len()).unwrap_or(u64::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::{export_playlist_file, import_playlist_file};
    use player_db::Database;
    use std::{fs, path::PathBuf};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moemusicplayer-playlist-{}",
                player_core::PlaylistId::new()
            ));
            fs::create_dir_all(&path).expect("create temporary test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn unicode_m3u8_import_and_export_keep_unmapped_file_path_in_rust() {
        let directory = TestDirectory::new();
        let media_directory = directory.0.join("夜の音樂");
        fs::create_dir_all(&media_directory).expect("create Unicode media directory");
        let media_path = media_directory.join("夜色 - Sample Track.mp3");
        fs::write(&media_path, b"test media bytes").expect("create Unicode media file");

        let playlist_path = directory.0.join("我的播放清單.m3u8");
        let playlist_text = format!(
            "#EXTM3U\r\n#PLAYLIST:夜色\r\n#EXTINF:215,夜色 - Sample Track\r\n{}\r\n",
            media_path.display()
        );
        fs::write(&playlist_path, playlist_text.as_bytes()).expect("write Unicode M3U8");

        let database_path = directory.0.join("library.sqlite3");
        let database = Database::open(&database_path).expect("open test database");
        let imported = import_playlist_file(&database, &playlist_path).expect("import M3U8");
        assert_eq!(imported.playlist.name, "夜色");
        assert_eq!(imported.playlist.entry_count, 1);
        assert_eq!(imported.matched_entries, 0);

        let page = database
            .get_playlist_page(imported.playlist.id, 0, 10)
            .expect("query imported playlist page")
            .expect("playlist exists");
        assert_eq!(page.total_count, 1);
        assert_eq!(page.items[0].track_id, None);
        assert!(!page.items[0].has_enabled_mapping);

        let legacy_playlist_path = directory.0.join("舊清單.m3u");
        fs::write(
            &legacy_playlist_path,
            "#EXTM3U\r\n#EXTINF:215,舊格式項目\r\n夜の音樂/夜色 - Sample Track.mp3\r\n",
        )
        .expect("write Unicode M3U");
        let legacy_imported =
            import_playlist_file(&database, &legacy_playlist_path).expect("import M3U");
        assert_eq!(legacy_imported.playlist.name, "舊清單");
        assert_eq!(legacy_imported.playlist.entry_count, 1);
        assert_eq!(legacy_imported.matched_entries, 0);

        let export_path = directory.0.join("匯出清單.m3u8");
        let exported = export_playlist_file(
            &database,
            &imported.playlist.id.to_string(),
            &export_path,
            "m3u8",
            None,
        )
        .expect("export M3U8");
        assert_eq!(exported.entry_count, 1);
        let exported_text = fs::read_to_string(export_path).expect("read exported M3U8");
        assert!(exported_text.contains("#PLAYLIST:夜色"));
        assert!(exported_text.contains("夜色 - Sample Track.mp3"));
        assert!(exported_text.contains("夜の音樂"));

        let relative_export_path = directory.0.join("相對匯出.m3u8");
        export_playlist_file(
            &database,
            &imported.playlist.id.to_string(),
            &relative_export_path,
            "m3u8",
            Some(&directory.0),
        )
        .expect("export relative M3U8 under shared root");
        let relative_text = fs::read_to_string(relative_export_path).expect("read relative M3U8");
        assert!(relative_text.contains("夜の音樂"));
        assert!(!relative_text.contains(&directory.0.to_string_lossy().to_string()));

        let m3u_export_path = directory.0.join("匯出清單.m3u");
        export_playlist_file(
            &database,
            &imported.playlist.id.to_string(),
            &m3u_export_path,
            "m3u",
            None,
        )
        .expect("export M3U");
        let m3u_export_bytes = fs::read(m3u_export_path).expect("read exported M3U");
        assert!(!m3u_export_bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let m3u_export_text = String::from_utf8(m3u_export_bytes).expect("M3U is UTF-8");
        assert!(m3u_export_text.contains("夜色 - Sample Track.mp3"));
    }
}
