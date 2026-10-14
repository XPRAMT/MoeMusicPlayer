# ADR 0011: SQLite-owned list query pipeline

## Status

Accepted — 2026-10-15T00:00:00+08:00

## Context

Track, playlist, queue, artist, album, and detail lists previously mixed front-end aggregation, grouping, search, sorting, and repeated paged reads. This caused avoidable renderer work and made list restoration sensitive to page timing. The UI still needs bounded data transfer and a virtualized DOM for large lists.

## Decision

- SQLite owns filtering/search, ordering, grouping/index boundaries, counts, and artist/album aggregation snapshots.
- Artist and album aggregate snapshots are computed once per query state. SQLite applies identity normalization, aggregate metrics, preferred display-name selection, order, grouping and JSON payload materialization in a single `INSERT ... SELECT`; Rust does not deserialize the full aggregate set and write it back to a temporary table. Subsequent pages read the retained result using `offset` and `limit`; pages do not rerun the aggregation.
- The renderer requests fixed pages of 1,000 entries. Page size is not derived from viewport height. Virtualization only bounds rendered DOM and does not perform data sorting, grouping, or database search.
- Quick-index boundaries are derived from the same database ordering as the list. The renderer maps database-provided group rows to virtual display rows; it does not scan pages to reconstruct groups.
- Album detail's numbered-disc and unknown-disc quick-index boundaries are part of the SQLite album-facets query for both sort directions. The renderer consumes those snapshots without summing disc counts or building group offsets.
- The album Disc quick-find panel displays numeric labels only (`1`, `2`, …); list group rows retain the full `DISC n` label used for navigation identity.
- The album-detail `#` column shows only the file's track number; Disc is already identified by its separate group row. A missing track number is shown as `—`.
- Album Disc grouping is enabled only when a Disc 2 or greater exists. When enabled, each numeric value including 0 and 1 has its own group; songs without a Disc value are `未知`. Ascending order places numeric groups low-to-high then unknown; descending puts unknown first then numeric groups high-to-low. Both list ordering and quick-index offsets follow that same SQL order.
- Missing/blank artist and album metadata each produce a distinct `未知` index entry, separate from a real artist or album named `未知`. SQLite orders unmarked entries last and emits the `未知` quick-index group. Artist detail queries accept an empty artist-name token as the unmarked category; album snapshots retain empty and whitespace aliases so opening the unmarked card returns the matching tracks.
- Queue operations preserve runtime traversal semantics, including duplicate entries and the current entry. SQLite sorts database-resolved sort candidates; the backend applies the resulting order to the traversal and persists the session.
- When Windows external and persistent catalogs are separate databases, each catalog applies search and field filtering in SQLite. A normal page reads all matching transient rows and one sorted persistent prefix with enough allowance for transient insertions. Locator retrieval and cross-catalog path resolution use JSON1 batches against indexed mappings, not per-track database locks. The union is staged in SQLite for final ICU4X ordering. Quick-index construction reads the complete match set once and asks SQLite to calculate group boundaries; it does not scan 256/1,000-row pages. Full candidates remain backend-only; renderer receives bounded pages.
- Artist and album identity equivalence uses registered SQLite scalar functions backed by NFKC, case folding, OpenCC simplified-to-traditional conversion, and a Japanese-kana guard. Display spelling and tie breaking are also selected by SQL, so the renderer no longer fetches all distinct tag facets or builds identity maps.
- Album-detail image candidate inspection may still page through track rows because perceptual cover selection is artwork processing, not list filtering/sorting/grouping.
- Single-file metadata failures during source sync remain isolated and must not abort reconciliation of other seen locators.

## Consequences

- Backend memory retains aggregate snapshots for active query states. Their lifetime must be bounded and invalidated when the corresponding source/filter state changes.
- IPC transfers only requested pages and aggregate summaries rather than full track collections.
- UI paging and virtual DOM are separate concerns; changing viewport dimensions must not change query page size or trigger full re-aggregation.
- SQLite tests cover SQL order, filtering, group boundaries, snapshot JSON payloads, artist/album identity normalization, queue traversal, and paged snapshots. Front-end tests cover offset paging, stable restoration, and virtual row mapping. Per project instruction, browser/Chrome/Playwright tests are not run unless the user explicitly asks.

## Validation

Validation completed at 2026-10-15T05:07:22+08:00: `npm run check` passed with 0 errors and 0 warnings; `node --test` passed 227 tests, including album-detail `#` labels and numeric-only Disc quick-find display; `cargo test -p player-db --lib --locked` passed 90 tests with 2 ignored, including unknown artist/album entries and navigable detail queries; `cargo test -p moemusicplayer --lib --locked -- --test-threads=1` passed 184 tests with 2 ignored; `cargo fmt --all -- --check` and `git diff --check` passed. Strict workspace Clippy passed during validation of the SQLite change. Browser/Chrome/Playwright tests were not run, per the user's instruction.
