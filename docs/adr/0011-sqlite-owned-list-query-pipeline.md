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
- Queue operations preserve runtime traversal semantics, including duplicate entries and the current entry. SQLite sorts database-resolved sort candidates; the backend applies the resulting order to the traversal and persists the session.
- When Windows external and persistent catalogs are separate databases, merge and comparison remain in Rust/backend code. Renderer never receives the full catalog for merging.
- Artist and album identity equivalence uses registered SQLite scalar functions backed by NFKC, case folding, OpenCC simplified-to-traditional conversion, and a Japanese-kana guard. Display spelling and tie breaking are also selected by SQL, so the renderer no longer fetches all distinct tag facets or builds identity maps.
- Album-detail image candidate inspection may still page through track rows because perceptual cover selection is artwork processing, not list filtering/sorting/grouping.
- Single-file metadata failures during source sync remain isolated and must not abort reconciliation of other seen locators.

## Consequences

- Backend memory retains aggregate snapshots for active query states. Their lifetime must be bounded and invalidated when the corresponding source/filter state changes.
- IPC transfers only requested pages and aggregate summaries rather than full track collections.
- UI paging and virtual DOM are separate concerns; changing viewport dimensions must not change query page size or trigger full re-aggregation.
- SQLite tests cover SQL order, filtering, group boundaries, snapshot JSON payloads, artist/album identity normalization, queue traversal, and paged snapshots. Front-end tests cover offset paging, stable restoration, and virtual row mapping. Per project instruction, browser/Chrome/Playwright tests are not run unless the user explicitly asks.

## Validation

For the current implementation on 2026-10-15: `npm run check` passed with 0 errors and 0 warnings; `node --test` passed; `cargo test -p player-db --lib --locked` passed 86 tests with 2 ignored; `cargo test -p moemusicplayer --lib --locked` passed 184 tests with 2 ignored; strict workspace Clippy and `cargo fmt --all -- --check` passed. Browser/Chrome/Playwright tests were not run, per the user's instruction.
