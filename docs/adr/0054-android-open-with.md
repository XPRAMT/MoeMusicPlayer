# Android: open audio files from other apps ("Open with")

Status: Accepted and implemented (unit tests only; nothing has run on a device yet)

## Context

On Windows an audio file opened from Explorer plays at once and the other files of its folder become the queue. On Android the app was not a target for audio files at all: a file manager, a browser download or a chat app could not offer it in the system's 「開啟方式」 chooser. The owner wants it to be one, with the same behaviour as Windows.

## Decision

### What the app declares

- Only `ACTION_VIEW` (no `ACTION_SEND`): the app does not appear in the share sheet. Opening a file is a different act from sending one.
- Three `intent-filter`s in `AndroidManifest.xml` (`DEFAULT` and `BROWSABLE`, schemes `content` and `file`):
  1. by type: `audio/*` and every MIME type of the formats the Android library accepts (`player_formats::android_open_mime_types()`; WavPack, Matroska, AC-3 and AMR gained their types in the table for this);
  2. by file name, for senders that give no type: `pathPattern` for every extension of `android_library_extensions()`, case-insensitive (`[fF][lL][aA][cC]`), in four variants (0 to 3 extra dots in the name) because Android's glob does not backtrack over a dot; a path pattern needs a host, so `host="*"`;
  3. the same file-name patterns with `mimeType="*/*"`, for senders that give `application/octet-stream` (a filter with a type does not match an intent without one, and the reverse).
- `plugins/tauri-plugin-media-index/tests/manifest_open_with.rs` reads the manifest and fails when the types or patterns differ from the format table, when a filter lacks a scheme or category, when a `SEND` action appears, or when the Kotlin MIME-to-extension map (`ExternalOpenRules.MIME_EXTENSIONS`, used to name a copy) differs from `extension_for_mime`.
- **The app cannot make itself the default.** Android has no API for it; declaring the filters only makes the app appear in the chooser. The user picks it there and chooses 「一律」 (always) in the system dialog; the default can be cleared in the system's app settings (Open by default). The app does not ask for it or claim it.
- `MainActivity` stays `launchMode="singleTask"`: a new `VIEW` intent goes to the running instance (`onNewIntent`), so the playback service and the page are not restarted.

### How the request reaches the player

The plugin (`MediaIndexPlugin`) reads the launch intent when it is created and every later one in `onNewIntent`, keeps only the newest in `ExternalOpenInbox`, and tells a running page with the window event `moe:android-external-open`. Replayed intents are ignored (`FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY`: Android brings the original intent back when the task is restored from Recents, and it would play the file again). The page takes the request with the command `android_external_open`: when the start page is decided, on the event, and when the app comes forward (an event can be lost while the WebView is paused). Calls are serialized (`createExternalOpenTaker`), so a request that arrives while another is being started is not lost. The command checks that the playback service exists before it takes the request, so a request is never taken and then dropped.

The remembered page (ADR 0053) must not cover the opened file: `settings_get_last_page` returns `externalLaunch` when the plugin holds a request, as it does on Windows when the process was started with a file. The start page is therefore decided before the request is taken. Like Windows, the app only starts playing; it does not open the Now Playing page.

### Resolving the URI

`file:` gives its path. A `content:` URI is mapped to a real path without asking the provider when it can be (`ExternalOpenRules.pathOfDocument`: external storage `<volume>:<relative>` ids, downloads `raw:/...` ids), then through the existing `documentPathFor` (the MediaStore `_data` column, media documents). The path is used only when the file exists and can be read. Otherwise the bytes are copied to `cache/external-open/` (one file; a new request clears it) and that copy is played alone: the folder of a copy holds nothing else, and a provider that hides the folder (another app's private files, or no All files access) gives no way to list it. This is the single-file fallback of the design.

### The queue

- Order: the other audio files of the folder that the format table accepts for the Android library (hidden files excluded), natural file-name order (`natural_title_cmp`, ties by raw name), the opened file always present and current. This is the order Windows gives the Explorer queue.
- A file the library already has (`resolve_track_id_for_locator` on its path) is its library track. Any other file is a track that exists only in this queue: a fresh `TrackId` and a `TrackSummary` kept in a process-wide registry (`external_open::registry()`), and nothing is written to SQLite. Windows uses an in-memory second database for this; that was not shared because the Android actor, the queue page, the artwork command and the notification all read one handle and one `TrackId` space, and a registry that a few call sites consult is much smaller than a second catalog behind all of them (the Windows catalog also drags the Windows-only service structure with it). The cost is that these tracks have no database row (below).
- The tags of the file that starts are read before it plays (the notification and the page need its title at once); the rest are read by a background thread, and until then the file name stands for the title.
- `PlaybackAction::OpenExternal` builds the queue in the actor like `SelectTrack` does and applies the same rule as every track change (ADR 0036): the file is loaded and acknowledged first, and a failure leaves the playing queue and track as they are. The registry keeps only the tracks of the queue that plays.

### What these tracks do not have

They are not in the library, so: no lyrics lookup, no song-information panel, no favorites, no listening statistics (the checkpoints are skipped before they reach the database), and the queue holding them is not saved as the playback session (the last library queue stays what a restart restores). A later version can store them if the owner wants lyrics for such files.

## Consequences

- The manifest, the format table and the Kotlin type map stay in step by a test; a new format gets its types and patterns by editing the table only.
- Not verified on a device: the chooser entry, the default choice, a cold start and a running app, the `content:` and `file:` senders of real file managers, browsers and chat apps, the copy fallback without All files access.
