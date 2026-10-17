//! Map legacy ExternalStorageProvider SAF URIs to real paths (ADR 0014 source migration).

use std::path::PathBuf;

const AUTHORITY_PREFIX: &str = "content://com.android.externalstorage.documents/";

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// `primary:Music/a.flac` -> `/storage/emulated/0/Music/a.flac`; `1A2B-3C4D:x` -> `/storage/1A2B-3C4D/x`.
pub fn external_storage_path_for_document_id(document_id: &str) -> Option<PathBuf> {
    let (volume, relative) = document_id.split_once(':')?;
    if volume.is_empty() || volume.contains('/') || relative.split('/').any(|part| part == "..") {
        return None;
    }
    let root = if volume == "primary" {
        "/storage/emulated/0".to_owned()
    } else {
        format!("/storage/{volume}")
    };
    let relative = relative.trim_matches('/');
    Some(if relative.is_empty() {
        PathBuf::from(root)
    } else {
        PathBuf::from(format!("{root}/{relative}"))
    })
}

/// Real path for an ExternalStorageProvider tree or document URI. The document ID wins over the
/// tree ID (`.../tree/<tree>/document/<doc>`). Other providers return `None`.
pub fn external_storage_path_for_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix(AUTHORITY_PREFIX)?;
    let segments = rest.split('/').collect::<Vec<_>>();
    let pick = |name: &str| {
        segments
            .iter()
            .position(|segment| *segment == name)
            .and_then(|index| segments.get(index + 1))
            .copied()
    };
    let encoded = pick("document").or_else(|| pick("tree"))?;
    external_storage_path_for_document_id(&percent_decode(encoded)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tree_and_document_uris_to_real_paths() {
        assert_eq!(
            external_storage_path_for_uri(
                "content://com.android.externalstorage.documents/tree/primary%3AMusic"
            ),
            Some(PathBuf::from("/storage/emulated/0/Music"))
        );
        assert_eq!(
            external_storage_path_for_uri(
                "content://com.android.externalstorage.documents/tree/primary%3AMusic/document/primary%3AMusic%2FMyLove.m3u8"
            ),
            Some(PathBuf::from("/storage/emulated/0/Music/MyLove.m3u8"))
        );
        assert_eq!(
            external_storage_path_for_uri(
                "content://com.android.externalstorage.documents/tree/primary%3APictures%2F%E9%8A%80%E7%8B%BC"
            ),
            Some(PathBuf::from("/storage/emulated/0/Pictures/銀狼"))
        );
        assert_eq!(
            external_storage_path_for_uri(
                "content://com.android.externalstorage.documents/tree/1A2B-3C4D%3A"
            ),
            Some(PathBuf::from("/storage/1A2B-3C4D"))
        );
    }

    #[test]
    fn rejects_other_providers_and_parent_segments() {
        assert_eq!(external_storage_path_for_uri("content://media/external/audio/media/1"), None);
        assert_eq!(
            external_storage_path_for_uri(
                "content://com.android.externalstorage.documents/tree/primary%3AMusic%2F..%2Fx"
            ),
            None
        );
    }
}
