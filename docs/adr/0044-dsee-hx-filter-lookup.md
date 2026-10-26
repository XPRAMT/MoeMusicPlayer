# DSEE HX filter lookup: beside the player first, then Sony Music Center

Status: Accepted

Decision date: 2026-10-27, Asia/Taipei (UTC+08:00)

## Context

DSEE HX upscaling (Windows only) runs Sony's 32-bit DirectShow filter `OmgDseeHxFilter.ax` inside our own 32-bit helper `dsee_hx_stream.exe` (`crates/player-audio-windows/dsee/dsee_hx_stream.cpp`, compiled by the crate's `build.rs` with the x86 MSVC compiler, embedded through `DSEE_HELPER_PATH` and written beside the player exe or into the temp directory at first use by `dsee::helper_exe`). The filter is Sony's; the project never includes, copies or distributes it. Until now the player and the helper both loaded it only from the Music Center install path `C:\Program Files (x86)\Sony\Music Center\Sony.Earth\OmgDseeHxFilter.ax`, so DSEE HX needed a full Music Center installation even when the user already had the filter files elsewhere (the portable player keeps everything in its own folder).

## Decision

- **Lookup order** (`dsee::filter_path`, the only resolver; `filter_candidates` lists the places, `first_existing_filter` is the pure, unit-tested pick):
  1. `OmgDseeHxFilter.ax` in the directory of the running player exe (`std::env::current_exe()`'s parent). Not the helper's directory: the helper may have been extracted to the temp directory.
  2. The Music Center install path above.
  3. Neither is a file → DSEE HX is unavailable and the track plays without it, with `INSTALL_NOTICE` (which names both places and says the project does not include or distribute the file).
- **The player passes the resolved absolute path to the helper** as its first command-line argument. The helper uses `wmain(argc, wchar_t** argv)`, so Unicode paths arrive intact; without an argument it falls back to the Music Center path. The helper checks the path is an existing file (not a directory), sets the DLL search directory to the filter's own directory (`SetDllDirectoryW`) and loads it with `LoadLibraryW`.
- **Companion DLLs**: the filter imports `omgutils_base.dll` (which imports `omgconfig.dll` → `libexpat.dll`) and the Visual C++ 2010 runtime (`msvcp100.dll`, `msvcr100.dll`). A filter placed beside the player therefore loads only if these are beside it too (or the runtime is installed system-wide); otherwise the helper reports `LoadLibrary failed` and the track plays without DSEE HX. The player does not search the Music Center directory for a beside-the-exe filter's dependencies, so the two locations never mix.
- **Packaging never picks the file up**: `scripts/package-windows-update.ps1` adds only `moemusicplayer.exe` to the update zip by explicit path, and `release/` is git-ignored, so a filter (or its DLLs) the developer keeps beside `release/moemusicplayer.exe` is not committed, zipped or uploaded.

## Consequences

- A user can run DSEE HX without installing Music Center by placing the filter and its companion DLLs beside the player; an installed Music Center keeps working unchanged.
- Verified with the crate's real-filter tests (`installed_filter_streams_a_short_tone`, `installed_filter_streams_24_and_32_bit_pcm`): from the Music Center path, and from beside the test exe with the companion DLLs copied there. With only the `.ax` copied, loading fails as described above.
- Not yet verified in the Release GUI with a filter beside `release/moemusicplayer.exe`.
