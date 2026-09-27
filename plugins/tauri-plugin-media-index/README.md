# Tauri plugin: media index

This crate adapts Android shared audio and user-selected document trees to the
`player_core` media-source records. Register it with
`tauri_plugin_media_index::init()` in the Tauri builder.

MediaStore sources use concrete collection URIs such as
`content://media/external_primary/audio/media`; the root's `SourceId` is supplied
by the application. SAF sources use the exact tree URI returned by
`pick_saf_tree()`, whose read permission is persisted by Android.

The adapter performs a full traversal for each scan. It returns `Complete` only
after the configured volume or document tree has been enumerated without row or
provider errors. A missing volume, revoked SAF grant, changing MediaStore
generation, or partial traversal produces a non-complete state so the database
must retain old mappings. MediaStore versions are available from API 29 and
generation values from API 30; API 28 and earlier use the external audio
collection and the legacy storage permission.

## Tests

Run Rust mapping tests with `cargo test -p tauri-plugin-media-index`. Android
adapter JVM tests are in `android/src/test`. Device behavior still requires
Android validation with granted MediaStore and SAF permissions.
