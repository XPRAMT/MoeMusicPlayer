# One Rust Simplified/Traditional converter with the OpenCC 1.4.2 dictionaries

Status: Accepted and implemented on 2026-10-22

Decision date: 2026-10-22, Asia/Taipei (UTC+08:00)

## Context

Three converters with three dictionary sets were in use:

- the frontend (`opencc-js` 1.4.2, OpenCC data 1.4.2) for the lyric display, the taskbar lyrics and the 繁簡 search
  variants of the library and of the online lyric search (`lyrics-traditional.js`, `search-variants.js`,
  `opencc-keep-zhi.js`);
- `ferrous-opencc` 0.4.0 (its own, older embedded dictionaries, synced 2026-04-18) for the artist/album identity keys
  in player-db (`S2t`, so OpenCC standard glyphs, no keep-只 rule);
- a generated character table `player-core/src/zh_t2s.rs` for script-insensitive lyric candidate scoring.

They disagreed (ferrous vs 1.4.2: 187 STPhrases, 29 TSPhrases and 291 TSCharacters first candidates differ), the
lyric display ran an asynchronous conversion after the lyrics arrived (a flash of the Simplified text, and a trackId
guard to drop late results), and the library and the lyric search depended on the frontend to send alternates.

## Decision

- **One crate, `crates/player-zh`**, used by player-core, player-db and src-tauri. No other code converts 繁簡.
  `ferrous-opencc`, `zh_t2s.rs`, the `opencc-js` runtime dependency, `opencc-keep-zhi.js`, `search-variants.js` and
  `src/opencc-js-dicts.d.ts` are removed.
- **Dictionaries**: the upstream text files of OpenCC tag `ver.1.4.2` (commit `025f371dc7`), unchanged, in
  `crates/player-zh/data/opencc-1.4.2/` with their Apache-2.0 `LICENSE` and a `SOURCE.md` listing versions and
  SHA-256. They are embedded with `include_str!` and parsed once, lazily (`OnceLock`), so they exist once in the
  binary. Upgrading means replacing the directory as a whole set (and the directory name in `src/lib.rs`), updating
  `SOURCE.md`, and running the fixture test. Only the files below are shipped.
- **The algorithm is OpenCC's**: max-match segmentation with the phrase dictionary (unmatched characters grouped
  into one segment), then each conversion stage per segment with longest-prefix matching, first candidate only. A
  merged table where the earlier dictionary wins on equal keys equals OpenCC's short-circuit group here.
- **Glyphs only, never wording.**
  - `to_traditional` (s2tw without regional phrases): stage 1 = STPhrases + STCharacters with the keep-只 overrides
    (every STPhrases entry whose value turns 只 into 隻 gets 只 restored at those positions; 只 never becomes 隻),
    stage 2 = TWVariantsPhrases + TWVariants. TWPhrases and `STPhrases_GeneratedFromRegionalPhrases.txt` are not
    used (they carry regional wording). TWVariantsPhrases is upstream s2tw's own stage-2 guard: identity entries
    such as 張棟樑 that keep TWVariants from changing names; it changes no wording.
  - `to_simplified` (tw2s without TSCharactersExt): stage 1 = TWVariantsRevPhrases + TWVariantsRev, stage 2 =
    TSPhrases + TSCharacters, segmentation with TSPhrases. **Why the reverse Taiwan variants run first**: the user's
    tags and queries are written in Taiwan glyphs, and TSCharacters is keyed by OpenCC standard Traditional. For 33
    of the 41 TWVariantsRev characters TSCharacters alone already gives the same result, but not for 8: Taiwan
    「著」 is also the particle 着 (看著你 → 看着你; without the stage it stayed 看著你), and 么/潀/痺/睪/簷/顎/鯰 are
    Taiwan forms whose Simplified is 幺/潨/痹/睾/檐/腭/鲇. TWVariantsRevPhrases keeps 著 where it means zhù (著名, 著作).
    TSCharactersExt is left out because upstream marks it `may_output_tofu`.
  - Text with Japanese kana (Unicode Script=Hiragana or Katakana) is never converted, as a whole line or field. The
    middle dot ・ and the prolonged sound mark ー are Script=Common and do not count, so 約翰・藍儂 still converts.
    The same rule now also applies to the identity keys (player-db used to treat the whole kana blocks, ・ included,
    as kana).
- **Lyric display**: `lyrics_get_track` and every other result that carries lyrics already return the lines converted:
  `LyricLineDto` has `textTraditional` / `translationTraditional`, present only when the Traditional spelling
  differs. The Now Playing page and the taskbar lyrics pick them synchronously (`lyricsForDisplay` in
  `src/lib/lyrics-traditional.js`) when the Simplified-to-Traditional preference is on. There is no second request,
  no flash and no stale result: the converted text arrives with the lines it belongs to, so the trackId and
  cancellation guards of the lyrics controller (unchanged) are sufficient. A future Android status-bar lyric reads
  the same fields. Cost: a few hundred bytes per song on the IPC, and the conversion runs for every lyric result even
  when the preference is off (microseconds per line).
- **Online lyric search** always searches the Simplified spelling only (QQ Music and NetEase index mainly
  Simplified): a manual query, or the track's title, artist and album for the automatic search, each converted with
  `to_simplified` in `lyrics_search`. Simplified is no longer converted to Traditional for a second round. The
  frontend sends only the query; `queryAlternates` is gone from `lyrics_search`.
- **Library search**: `search_terms_json` in player-db adds `player_zh::search_alternates(query)` (the Simplified and
  the Taiwan-Traditional spelling, without duplicates, none for text without Han or with kana) to every list,
  position, quick-index and queue query. The frontend no longer sends `queryAlternates`; the commands still accept
  it (ignored when empty) so a queue source saved before this change stays valid.
- **Identity keys** (`music_identity_key` / `music_identity_traditional`) use `to_traditional`. They exist only in
  per-connection TEMP tables and are recomputed in every session, so no migration or schema version bump is needed:
  the first start of the new build groups with the new dictionaries. Verified with the in-memory test database only.
- **Lyric candidate scoring** folds single characters with `fold_to_simplified` (TWVariantsRev, then TSCharacters).
- **`scripts/i18n-catalog.mjs` keeps `opencc-js` as a devDependency.** It generates the Simplified interface catalog
  with OpenCC `twp → cn`, which deliberately converts wording (軟體 → 软件) and runs only at development time;
  rewriting it in Rust would need the regional phrase dictionaries in the shipped crate. Nothing in the bundle imports
  `opencc-js` any more.
- **Golden fixture**: `tests/fixtures/zh-convert-opencc-js.json` was recorded with the former opencc-js conversion
  before removal (`node scripts/generate-zh-convert-fixture.mjs`, which inlines the old rules): 174 inputs covering
  只 phrases, kana, Taiwan glyphs, lyric phrases, titles and artists, one-to-many characters (头发/发现, 干/乾/幹 …)
  and Traditional input. `crates/player-zh/tests/opencc_js_fixture.rs` reproduces all of it except the 18 entries in
  `tests/fixtures/zh-convert-differences.json`, each with the new value and the reason: the lyric display is
  identical for all 174; 17 search-Traditional entries now use Taiwan glyphs (為/裡/著/群/眾/床/峰/秘/吃/麵 instead
  of 爲/裏/着/羣/衆/牀/峯/祕/喫/麪), because search and identity now use the same s2tw as the display; one
  Simplified entry is 看著你 → 看着你 (the TWVariantsRev stage).

## Consequences

- One dictionary set (about 1.1 MB of text in the binary, parsed on first use) instead of three; the frontend bundle
  no longer carries the opencc-js dictionaries.
- Searching the library for 里面 now also matches tags written 裡面 (Taiwan) but not 裏面 (OpenCC standard); before it
  was the reverse.
- Lyric search finds only what the providers return for the Simplified spelling; a Traditional-only provider entry
  is no longer searched for separately.
- Conversion changes now need a Rust build; the fixture test shows any behaviour change against opencc-js.
