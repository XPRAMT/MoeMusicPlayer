# Remember the page the app closed on

Status: Accepted and implemented (unit tests only; nothing has run in the Release app or on a device yet)

## Context

Every start opened the library. The owner wants the app to open the page it was on when it closed, on Windows and Android with the same logic: the main page, the album or artist detail that was open, and whether the Now Playing page was open. Not the list scroll position, not the search text, not dialogs.

## Decision

### What is remembered

One object, `settings.json` → `lastPage` (`LastPageSettings` in `settings.rs`, mirrored by `src/lib/start-page.js`):

- `view`: one of the navigation views `library`, `artists`, `artist-detail`, `albums`, `album-detail`, `playlists`, `queue`, `settings` (the `View` type of `App.svelte`).
- Only the field the view needs is kept: `playlistId` for `playlists`, `artistKey` / `albumKey` (the identity keys the history routes already use) for the detail views, `settingsSection` (the stable tab id) for `settings`. The rest is null.
- `nowPlaying`: whether the Now Playing overlay was open. The page underneath the overlay is `view` (`activeView` keeps it while the overlay is open).

The default is `library` with everything else empty. Rust and JS hold the same view and tab lists and the same default; `tests/start-page.test.mjs` reads `settings.rs` and fails when they differ, and also checks them against the `View` and `SettingsSection` types in `App.svelte`.

### Reading is lenient, in both languages

A missing, damaged or non-object value, an unknown `view` (from another version), or a view whose target key is missing or over-long is the default page, never an error (`lenient_last_page` / `normalizeStartPage`; same rule in `normalized()` for the value the page sends). An unknown settings tab is dropped (the view stays). Fields of another type are ignored.

### Saving

A `$effect` in `App.svelte` turns the navigation state into the value (`startPageFromState`) and hands it to a debounced saver (`createStartPageSaver`, 400 ms of quiet; only the latest value is written; a value equal to what is stored is never written). The saver is flushed at once when the page becomes hidden, on `pagehide` and when the component is destroyed, because the Android process can be killed after the page is hidden. Saving starts only after the restore settled, so the default page of a booting app never overwrites the remembered one; the stored page is the saver's baseline, so a restored page is not written back. While the Now Playing page is still waiting for its track the saved flag stays set. Commands: `settings_get_last_page` (also returns `externalLaunch`) and `settings_set_last_page`.

### Restoring

At the start of `initializeTauri` the value is requested before anything else is loaded.

- Pages that need no lookup (`library`, `artists`, `albums`, `queue`, `settings` + tab) are applied before capabilities and the library load, so the library page does not show first.
- Playlist, artist and album pages need their target: the playlist list (`playlist_list`), `library_resolve_artist_index_entry` / `library_resolve_album_index_entry` by key. The check runs once the library is ready; if the target is gone, or the library is not ready, the app stays on the default page and says nothing (`resolveStartPage`, which also drops the Now Playing flag). The detail is opened by setting the same state the list uses (no history entry is pushed; Back goes to the index like it does for a detail opened by name).
- A late restore is skipped when the user already moved to another page (`lateRestoreStillWanted`).
- The Now Playing page opens once the playback snapshot names a current track (the snapshot can arrive before the saved session is restored, so it waits up to 5 s, then gives up; `nowPlayingRestoreAction`). `openNowPlaying` records the restored page as the return view.

### Launch intents win

The remembered page is not restored when the run was started to do something else (`planStartRestore`, `externalLaunch`). Today the only such intent is Windows opening an audio file from Explorer (`initial_audio_path` at process start, kept in the managed `LaunchIntent` state; a file forwarded to an already running instance never restores anything, since a restore only happens at start). Android has no launch intent that navigates: the notification and launcher open `MainActivity` without extras, and a tap while the app is running is not a start. After an external launch the app is on the default page and what the user does next becomes the remembered page.

## Consequences

- No scroll position is stored, so a restored list starts at the top; the album and artist detail restore without `preserveIndex`.
- A flush at window close on Windows may not finish if the page is torn down at once; the previous value (at most 400 ms older) stays.
- The page the app was on in a run that was force-killed within the debounce window is not remembered.

## Not verified

Release app (CDP) and device runs: the first paint of a restored page without a library flash, the playlist / album / artist restore against real data, the Now Playing wait on a slow Android session restore, Android process death after the page is hidden, Explorer "open with" starting on the default page.
