use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::{ItemKey, TagExt};
use lofty::tag::{MergeTag, SplitTag, Tag, TagType};
use serde::{Deserialize, Serialize};

use crate::ape_cover::{ape_front_cover_of_path, is_ape_family, read_ape_tag};
use crate::cover::{prepare_cover, CoverOptions, PreparedCover};

/// Common editable fields. Empty string means "absent".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TagFields {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub composer: String,
    pub genre: String,
    pub track_number: String,
    pub track_total: String,
    pub disc_number: String,
    pub disc_total: String,
    pub date: String,
    pub lyrics: String,
}

type FieldAccess = (
    ItemKey,
    fn(&TagFields) -> &String,
    fn(&mut TagFields) -> &mut String,
);

const FIELD_KEYS: [FieldAccess; 12] = [
    (ItemKey::TrackTitle, |f| &f.title, |f| &mut f.title),
    (ItemKey::TrackArtist, |f| &f.artist, |f| &mut f.artist),
    (ItemKey::AlbumTitle, |f| &f.album, |f| &mut f.album),
    (
        ItemKey::AlbumArtist,
        |f| &f.album_artist,
        |f| &mut f.album_artist,
    ),
    (ItemKey::Composer, |f| &f.composer, |f| &mut f.composer),
    (ItemKey::Genre, |f| &f.genre, |f| &mut f.genre),
    (
        ItemKey::TrackNumber,
        |f| &f.track_number,
        |f| &mut f.track_number,
    ),
    (
        ItemKey::TrackTotal,
        |f| &f.track_total,
        |f| &mut f.track_total,
    ),
    (
        ItemKey::DiscNumber,
        |f| &f.disc_number,
        |f| &mut f.disc_number,
    ),
    (ItemKey::DiscTotal, |f| &f.disc_total, |f| &mut f.disc_total),
    (ItemKey::RecordingDate, |f| &f.date, |f| &mut f.date),
    (ItemKey::Lyrics, |f| &f.lyrics, |f| &mut f.lyrics),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioProperties {
    pub duration_ms: u64,
    pub overall_bitrate_kbps: Option<u32>,
    pub audio_bitrate_kbps: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub channels: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverInfo {
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDetails {
    pub path: String,
    pub file_name: String,
    pub file_size: u64,
    pub modified_unix_ms: Option<u64>,
    /// File creation time when the platform reports it (Windows); `None` otherwise.
    pub created_unix_ms: Option<u64>,
    pub format: String,
    /// e.g. "ID3v2.3", "VorbisComments", "MP4 ilst"; empty when no tag.
    pub tag_format: String,
    pub read_only: bool,
    pub read_only_reason: Option<String>,
    /// The file is on a remote server, not on this device (it has no local folder to open).
    #[serde(default)]
    pub remote: bool,
    pub fields: TagFields,
    pub properties: AudioProperties,
    pub cover: Option<CoverInfo>,
}

/// Staged edit applied on Save. `fields` is the full desired state of the common fields
/// (empty = remove); only fields that differ from the file are touched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagEdit {
    pub fields: TagFields,
    /// Replacement cover image bytes (any decodable image).
    #[serde(default)]
    pub replace_cover: Option<Vec<u8>>,
    #[serde(default)]
    pub remove_cover: bool,
    #[serde(default)]
    pub cover_options: CoverOptions,
}

/// DSF/DFF are read-only (ADR 0013 D10); the capability table decides.
fn is_read_only_format(path: &Path) -> bool {
    !player_formats::tag_write_supported(path)
}

fn id3v2_version(path: &Path) -> Option<u8> {
    let mut header = [0u8; 4];
    fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    (&header[..3] == b"ID3").then_some(header[3])
}

fn file_meta(path: &Path) -> Result<(u64, Option<u64>, Option<u64>), String> {
    let meta = fs::metadata(path).map_err(|e| format!("無法讀取檔案資訊：{e}"))?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    let created = match meta.created() {
        Ok(time) => time
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64),
        Err(error) => {
            // Not every file system keeps a creation time (Android's user storage, a FUSE mount, reports none).
            eprintln!("[details] no creation time for {}: {error}", path.display());
            None
        }
    };
    Ok((meta.len(), modified, created))
}

/// Human-readable tag format label (e.g. "Vorbis Comments" instead of `VorbisComments`).
fn friendly_tag_type(tag_type: TagType) -> String {
    match tag_type {
        TagType::VorbisComments => "Vorbis Comments".to_owned(),
        TagType::Mp4Ilst => "MP4 (iTunes ilst)".to_owned(),
        TagType::Ape => "APEv2".to_owned(),
        TagType::Id3v1 => "ID3v1".to_owned(),
        TagType::RiffInfo => "RIFF INFO".to_owned(),
        TagType::AiffText => "AIFF Text".to_owned(),
        other => format!("{other:?}"),
    }
}

/// ID3v2 cannot hold `ItemKey::Lyrics`; lyrics (including LRC text) go to USLT there.
fn effective_key(key: ItemKey, tag_type: TagType) -> ItemKey {
    if key == ItemKey::Lyrics && tag_type == TagType::Id3v2 {
        ItemKey::UnsyncLyrics
    } else {
        key
    }
}

/// Tag type we read from and write to. WAV uses its ID3v2 chunk (RIFF INFO lacks lyrics,
/// album artist, etc.).
fn target_tag_type(file: &lofty::file::TaggedFile) -> TagType {
    if file.file_type() == FileType::Wav {
        TagType::Id3v2
    } else {
        file.primary_tag_type()
    }
}

fn front_cover(tag: &Tag) -> Option<&Picture> {
    tag.pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
}

/// Read the front cover bytes and mime type (for export).
pub fn read_cover(path: &Path) -> Result<Option<(Vec<u8>, String)>, String> {
    let dsd_tag;
    let file;
    let tag = if is_dsd(path) {
        dsd_tag = player_dsd::id3::read_dsf(path)
            .map_err(|e| e.to_string())?
            .1;
        dsd_tag.as_ref()
    } else if is_read_only_format(path) {
        return Ok(None);
    } else {
        file = lofty::read_from_path(path).map_err(|e| format!("讀取標籤失敗：{e}"))?;
        file.primary_tag().or_else(|| file.first_tag())
    };
    let ape_cover = match tag.and_then(front_cover) {
        Some(_) => None,
        None => ape_front_cover_of_path(path),
    };
    let cover = tag.and_then(front_cover).or(ape_cover.as_ref());
    Ok(cover.map(|p| {
        let mime = p
            .mime_type()
            .map(|m| m.as_str().to_owned())
            .unwrap_or_else(|| sniff_mime(p.data()).to_owned());
        (p.data().to_vec(), mime)
    }))
}

fn sniff_mime(data: &[u8]) -> &'static str {
    match image::guess_format(data) {
        Ok(image::ImageFormat::Png) => "image/png",
        Ok(image::ImageFormat::WebP) => "image/webp",
        Ok(image::ImageFormat::Gif) => "image/gif",
        Ok(image::ImageFormat::Bmp) => "image/bmp",
        _ => "image/jpeg",
    }
}

fn is_dsd(path: &Path) -> bool {
    player_formats::decode_engine(path) == player_formats::DecodeEngine::Dsd
}

/// DSF properties from the file header and tag fields from its trailing ID3v2 tag (read-only).
fn fill_dsf_details(details: &mut TrackDetails, path: &Path) -> Result<(), String> {
    let (info, tag) = player_dsd::id3::read_dsf(path).map_err(|e| e.to_string())?;
    details.format = info.rate_name();
    let duration_ms = info.duration().as_millis() as u64;
    let bitrate_kbps = info.sample_rate.saturating_mul(u32::from(info.channels)) / 1000;
    details.properties = AudioProperties {
        duration_ms,
        overall_bitrate_kbps: Some(bitrate_kbps),
        audio_bitrate_kbps: Some(bitrate_kbps),
        sample_rate: Some(info.sample_rate),
        bit_depth: Some(1),
        channels: u8::try_from(info.channels).ok(),
    };
    if let Some(tag) = tag.as_ref() {
        details.tag_format = "ID3v2".to_owned();
        for (key, _, get_mut) in FIELD_KEYS.iter() {
            if let Some(value) = tag.get_string(effective_key(*key, tag.tag_type())) {
                *get_mut(&mut details.fields) = value.to_owned();
            }
        }
        if let Some(pic) = front_cover(tag) {
            let (width, height) = image::load_from_memory(pic.data())
                .map(|i| (i.width(), i.height()))
                .unwrap_or((0, 0));
            details.cover = Some(CoverInfo {
                mime: pic
                    .mime_type()
                    .map(|m| m.as_str().to_owned())
                    .unwrap_or_else(|| sniff_mime(pic.data()).to_owned()),
                width,
                height,
                bytes: pic.data().len() as u64,
            });
        }
    }
    Ok(())
}

/// Short uppercase container name shown in the details panel (matches the app's codec labels:
/// MP3, FLAC, WAV, ...). MP4 audio keeps its file extension (M4A / MP4 / M4B).
fn format_name(file_type: FileType, path: &Path) -> String {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    match file_type {
        FileType::Mpeg => "MP3".to_owned(),
        FileType::Flac => "FLAC".to_owned(),
        FileType::Wav => "WAV".to_owned(),
        FileType::Aiff => "AIFF".to_owned(),
        FileType::Aac => "AAC".to_owned(),
        FileType::Ape => "APE".to_owned(),
        FileType::Opus => "OPUS".to_owned(),
        FileType::Vorbis => "OGG".to_owned(),
        FileType::Speex => "SPEEX".to_owned(),
        FileType::WavPack => "WV".to_owned(),
        FileType::Mpc => "MPC".to_owned(),
        FileType::Mp4 if !extension.is_empty() => extension,
        FileType::Mp4 => "MP4".to_owned(),
        other => format!("{other:?}").to_uppercase(),
    }
}

pub fn read_details(path: &Path) -> Result<TrackDetails, String> {
    let (file_size, modified_unix_ms, created_unix_ms) = file_meta(path)?;
    let mut details = TrackDetails {
        path: path.to_string_lossy().into_owned(),
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file_size,
        modified_unix_ms,
        created_unix_ms,
        format: path
            .extension()
            .map(|e| e.to_string_lossy().to_uppercase())
            .unwrap_or_default(),
        tag_format: String::new(),
        read_only: false,
        read_only_reason: None,
        remote: false,
        fields: TagFields::default(),
        properties: AudioProperties::default(),
        cover: None,
    };
    if is_dsd(path) {
        // DSF: properties from the header, tag from the trailing ID3v2; editable (ADR 0013 D10, revised).
        fill_dsf_details(&mut details, path)?;
        return Ok(details);
    }
    if is_read_only_format(path) {
        details.read_only = true;
        details.read_only_reason = Some("DFF 目前僅支援唯讀（ADR 0013 D10）".into());
        return Ok(details);
    }
    let file = lofty::read_from_path(path).map_err(|e| format!("讀取標籤失敗：{e}"))?;
    details.format = format_name(file.file_type(), path);
    let p = file.properties();
    details.properties = AudioProperties {
        duration_ms: p.duration().as_millis() as u64,
        overall_bitrate_kbps: p.overall_bitrate(),
        audio_bitrate_kbps: p.audio_bitrate(),
        sample_rate: p.sample_rate(),
        bit_depth: p.bit_depth(),
        channels: p.channels(),
    };
    let target = target_tag_type(&file);
    if let Some(tag) = file
        .tag(target)
        .or_else(|| file.primary_tag())
        .or_else(|| file.first_tag())
    {
        details.tag_format = match (tag.tag_type(), id3v2_version(path)) {
            (TagType::Id3v2, Some(v)) => format!("ID3v2.{v}"),
            (TagType::Id3v2, None) => "ID3v2".to_owned(),
            (t, _) => friendly_tag_type(t),
        };
        for (key, _, get_mut) in FIELD_KEYS.iter() {
            if let Some(value) = tag.get_string(effective_key(*key, tag.tag_type())) {
                *get_mut(&mut details.fields) = value.to_owned();
            }
        }
        let ape_cover = match front_cover(tag) {
            Some(_) => None,
            None => ape_front_cover_of_path(path),
        };
        let cover = front_cover(tag).or(ape_cover.as_ref());
        if let Some(pic) = cover {
            let (width, height) = image::load_from_memory(pic.data())
                .map(|i| (i.width(), i.height()))
                .unwrap_or((0, 0));
            details.cover = Some(CoverInfo {
                mime: pic
                    .mime_type()
                    .map(|m| m.as_str().to_owned())
                    .unwrap_or_else(|| sniff_mime(pic.data()).to_owned()),
                width,
                height,
                bytes: pic.data().len() as u64,
            });
        }
    }
    Ok(details)
}

fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".moetmp-{name}"))
}

/// Apply a staged edit atomically: copy to a sibling temp file (`.moetmp-<name>`, same extension so the format is detected), write tags there (MP3 as
/// ID3v2.4), verify by re-reading, then rename over the original. The file's mtime is
/// updated normally. On failure the original is untouched.
pub fn write_edit(path: &Path, edit: &TagEdit) -> Result<TrackDetails, String> {
    if is_read_only_format(path) {
        return Err("DFF 目前僅支援唯讀".into());
    }
    let cover = match &edit.replace_cover {
        Some(bytes) => Some(prepare_cover(bytes, edit.cover_options)?),
        None => None,
    };
    let temp = temp_path_for(path);
    let result = (|| {
        if is_dsd(path) {
            return write_dsf_edit(path, &temp, edit, cover.as_ref());
        }
        fs::copy(path, &temp).map_err(|e| format!("建立暫存檔失敗：{e}"))?;
        if let Some(file_type) = FileType::from_path(&temp).filter(|t| is_ape_family(*t)) {
            write_ape_edit(&temp, file_type, edit, cover.as_ref())?;
            lofty::read_from_path(&temp).map_err(|e| format!("寫入後驗證失敗：{e}"))?;
            fs::rename(&temp, path).map_err(|e| format!("取代原檔失敗（檔案可能被占用）：{e}"))?;
            return Ok(());
        }
        let mut file = lofty::read_from_path(&temp).map_err(|e| format!("讀取標籤失敗：{e}"))?;
        let tag_type = target_tag_type(&file);
        if file.tag(tag_type).is_none() {
            file.insert_tag(Tag::new(tag_type));
        }
        let tag = file.tag_mut(tag_type).ok_or("無法建立標籤")?;
        apply_edit_to_tag(tag, tag_type, edit, cover.as_ref())?;
        // Default WriteOptions write ID3v2.4 (decision D3: always upgrade MP3 to v2.4).
        tag.save_to_path(&temp, WriteOptions::default())
            .map_err(|e| format!("寫入標籤失敗：{e}"))?;
        // Verify the temp file still parses before replacing the original.
        lofty::read_from_path(&temp).map_err(|e| format!("寫入後驗證失敗：{e}"))?;
        fs::rename(&temp, path).map_err(|e| format!("取代原檔失敗（檔案可能被占用）：{e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    read_details(path)
}

/// WavPack and Monkey's Audio: edit the file's own `ApeTag`. lofty's generic `Tag` neither carries the covers of an
/// APEv2 tag nor writes pictures back, so the existing cover items stay in the `ApeTag` (and are dropped only when the
/// edit replaces or removes the cover) and the new cover is added as a `Cover Art (Front)` item.
fn write_ape_edit(
    temp: &Path,
    file_type: FileType,
    edit: &TagEdit,
    cover: Option<&PreparedCover>,
) -> Result<(), String> {
    let mut ape = {
        let mut file = fs::File::open(temp).map_err(|e| format!("讀取標籤失敗：{e}"))?;
        read_ape_tag(&mut std::io::BufReader::new(&mut file), file_type).unwrap_or_default()
    };
    if edit.remove_cover || cover.is_some() {
        ape.remove("Cover Art (Front)");
        ape.remove("Cover Art (Other)");
    }
    let (remainder, mut tag) = ape.split_tag();
    apply_edit_to_tag(&mut tag, TagType::Ape, edit, cover)?;
    remainder
        .merge_tag(tag)
        .save_to_path(temp, WriteOptions::default())
        .map_err(|e| format!("寫入標籤失敗：{e}"))
}

/// Apply the staged field and cover changes to a tag (shared by every format).
fn apply_edit_to_tag(
    tag: &mut Tag,
    tag_type: TagType,
    edit: &TagEdit,
    cover: Option<&PreparedCover>,
) -> Result<(), String> {
    for (key, get, _) in FIELD_KEYS.iter() {
        let key = &effective_key(*key, tag_type);
        let desired = get(&edit.fields);
        let current = tag.get_string(*key).unwrap_or("");
        if desired == current {
            continue;
        }
        if desired.is_empty() {
            tag.remove_key(*key);
        } else if !tag.insert_text(*key, desired.clone()) {
            return Err(format!("此格式不支援欄位 {key:?}"));
        }
    }
    if edit.remove_cover || cover.is_some() {
        tag.remove_picture_type(PictureType::CoverFront);
        // Single-cover model: drop an untyped "Other" cover too so the new one is used.
        tag.remove_picture_type(PictureType::Other);
    }
    if let Some(cover) = cover {
        let mime = if cover.mime == "image/jpeg" {
            MimeType::Jpeg
        } else {
            MimeType::Png
        };
        tag.push_picture(
            Picture::unchecked(cover.data.clone())
                .pic_type(PictureType::CoverFront)
                .mime_type(mime)
                .build(),
        );
    }
    Ok(())
}

/// DSF: rewrite the file with its trailing ID3v2.4 tag replaced (audio untouched, header pointers fixed), verify
/// the result, then replace the original.
fn write_dsf_edit(
    path: &Path,
    temp: &Path,
    edit: &TagEdit,
    cover: Option<&PreparedCover>,
) -> Result<(), String> {
    let (_, existing) =
        player_dsd::id3::read_dsf(path).map_err(|e| format!("讀取標籤失敗：{e}"))?;
    let mut tag = existing.unwrap_or_else(|| Tag::new(TagType::Id3v2));
    apply_edit_to_tag(&mut tag, TagType::Id3v2, edit, cover)?;
    player_dsd::id3::rewrite_dsf_with_tag(path, temp, &tag)
        .map_err(|e| format!("寫入標籤失敗：{e}"))?;
    // Verify: the new file parses and carries the title we wrote, and the stream is as long as before.
    let before = player_dsd::id3::read_dsf(path)
        .map_err(|e| format!("讀取標籤失敗：{e}"))?
        .0;
    let (after, written) =
        player_dsd::id3::read_dsf(temp).map_err(|e| format!("寫入後驗證失敗：{e}"))?;
    if after != before {
        return Err("寫入後驗證失敗：音訊資訊與原檔不一致".into());
    }
    let wrote_title = written
        .as_ref()
        .and_then(|tag| tag.get_string(ItemKey::TrackTitle))
        .unwrap_or("");
    if wrote_title != edit.fields.title {
        return Err("寫入後驗證失敗：標題與預期不符".into());
    }
    fs::rename(temp, path).map_err(|e| format!("取代原檔失敗（檔案可能被占用）：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod format_name_tests {
    use super::*;

    #[test]
    fn container_names_are_short_uppercase() {
        assert_eq!(format_name(FileType::Mpeg, Path::new("a.mp3")), "MP3");
        assert_eq!(format_name(FileType::Flac, Path::new("a.flac")), "FLAC");
        assert_eq!(format_name(FileType::Wav, Path::new("a.wav")), "WAV");
        assert_eq!(format_name(FileType::Mp4, Path::new("a.m4a")), "M4A");
        assert_eq!(format_name(FileType::Mp4, Path::new("a")), "MP4");
    }
}
