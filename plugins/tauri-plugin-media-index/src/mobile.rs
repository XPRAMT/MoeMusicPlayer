use std::sync::Arc;

use player_core::{
    LibraryRoot, MediaLocator, MediaSourceError, MediaSourceKind, MediaTrackRecord, SourceScan,
    SourceScanState, TrackMetadata, TrackMetadataError,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::adapter::{
    into_core_metadata, looks_like_tree_uri, media_store_volume, scan_from_response,
};
use crate::models::{
    BooleanResponse, MetadataResponse, PermissionResponse, SafTreeResponse, ScanResponse,
    VolumesResponse,
};
use crate::{MediaStoreVolume, MediaStoreVolumes, Result};

#[derive(Clone)]
pub(crate) struct PlatformMediaIndex<R: Runtime>(Arc<PluginHandle<R>>);

impl<R: Runtime> PlatformMediaIndex<R> {
    pub(crate) fn init<C: DeserializeOwned>(
        _app: &AppHandle<R>,
        api: PluginApi<R, C>,
    ) -> Result<Self> {
        let handle =
            api.register_android_plugin("com.moemusicplayer.mediaindex", "MediaIndexPlugin")?;
        Ok(Self(Arc::new(handle)))
    }

    pub(crate) fn scan(&self, root: &LibraryRoot) -> SourceScan {
        let request = match (&root.kind, &root.locator) {
            (MediaSourceKind::AndroidMediaStore, MediaLocator::ContentUri(uri)) => {
                match media_store_volume(uri) {
                    Some(volume_name) => (
                        "scanMediaStore",
                        json!({
                            "sourceId": root.id.to_string(),
                            "volumeName": volume_name,
                        }),
                    ),
                    None => {
                        return unavailable_scan(
                            root,
                            "MediaStore source URI must identify a concrete media volume",
                        );
                    }
                }
            }
            (MediaSourceKind::AndroidSaf, MediaLocator::ContentUri(uri)) => {
                if !looks_like_tree_uri(uri) {
                    return unavailable_scan(root, "SAF source is not a document-tree URI");
                }
                (
                    "scanSafTree",
                    json!({"sourceId": root.id.to_string(), "treeUri": uri}),
                )
            }
            (MediaSourceKind::AndroidMediaStore | MediaSourceKind::AndroidSaf, _) => {
                return unavailable_scan(root, "Android source requires a content URI");
            }
            _ => return unavailable_scan(root, "source kind is not supported by Android"),
        };

        let response = self
            .0
            .run_mobile_plugin::<ScanResponse>(request.0, request.1);
        match response {
            Ok(response) => scan_from_response(root, response),
            Err(error) => unavailable_scan(root, &format!("Android source query failed: {error}")),
        }
    }

    pub(crate) fn read_metadata(
        &self,
        record: &MediaTrackRecord,
    ) -> std::result::Result<TrackMetadata, TrackMetadataError> {
        let MediaLocator::ContentUri(uri) = &record.locator else {
            return Err(TrackMetadataError {
                message: "Android media metadata requires a content URI".into(),
            });
        };
        let response = self
            .0
            .run_mobile_plugin::<MetadataResponse>("readMetadata", json!({ "contentUri": uri }))
            .map_err(|error| TrackMetadataError {
                message: error.to_string(),
            })?;
        if let Some(message) = response.error {
            return Err(TrackMetadataError { message });
        }
        response
            .metadata
            .map(into_core_metadata)
            .ok_or_else(|| TrackMetadataError {
                message: "Android returned no metadata for the content URI".into(),
            })
    }

    pub(crate) fn list_media_store_volumes(&self) -> Result<MediaStoreVolumes> {
        let response = self
            .0
            .run_mobile_plugin::<VolumesResponse>("mediaStoreVolumes", ())?;
        Ok(MediaStoreVolumes {
            api_level: response.api_level,
            volumes: response
                .volumes
                .into_iter()
                .map(|volume| MediaStoreVolume {
                    volume_name: volume.volume_name,
                    version: volume.version,
                    generation: volume.generation,
                })
                .collect(),
        })
    }

    pub(crate) fn request_media_store_permission(&self) -> Result<bool> {
        let response = self
            .0
            .run_mobile_plugin::<PermissionResponse>("requestMediaReadPermission", ())?;
        Ok(response.granted)
    }

    pub(crate) fn pick_saf_tree(&self) -> Result<Option<String>> {
        let response = self
            .0
            .run_mobile_plugin::<SafTreeResponse>("pickSafTree", ())?;
        Ok(response.uri)
    }

    pub(crate) fn has_saf_permission(&self, uri: &str) -> Result<bool> {
        let response = self
            .0
            .run_mobile_plugin::<BooleanResponse>("hasSafPermission", json!({"treeUri": uri}))?;
        Ok(response.granted)
    }

    pub(crate) fn release_saf_tree(&self, uri: &str) -> Result<()> {
        let _: serde_json::Value = self
            .0
            .run_mobile_plugin("releaseSafTree", json!({"treeUri": uri}))?;
        Ok(())
    }
}

fn unavailable_scan(root: &LibraryRoot, reason: &str) -> SourceScan {
    SourceScan {
        source_id: root.id,
        state: SourceScanState::Unavailable {
            reason: reason.to_owned(),
        },
        tracks: Vec::new(),
        errors: vec![MediaSourceError {
            source_item_id: None,
            message: reason.to_owned(),
        }],
    }
}
