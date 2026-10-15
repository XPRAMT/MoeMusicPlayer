# ADR 0012: Stable album-detail scrollport during cover collapse

## Status

Accepted — 2026-10-15T00:00:00+08:00

## Context

The album-detail cover shrinks while the track list scrolls. When the heading and list were separate flex siblings, shrinking the heading enlarged the list's `clientHeight` while its full `scrollHeight` remained constant. The native scrollbar therefore changed thumb size during a gesture, and its scroll range changed under the pointer.

## Decision

- Keep one fixed-size native scrollport covering the album-detail viewport. Its scrollable extent remains the full heading reserve, sortable track header, and virtual track content.
- Draw the album heading and sortable track header as overlays above the virtual rows. As the cover collapses, the overlays expose more of the same scrollport; neither its `clientHeight` nor its `scrollHeight` is changed by the collapse.
- Preserve a logical track-list scroll offset separately from the native scroll offset. While the cover is collapsing, one logical pixel advances the native scroll position by two pixels; after the cover reaches 64 CSS px, native and logical movement match. Upward movement keeps the cover size until the logical list returns to its top.
- Keep saved list positions, database paging, grouping, sorting, and quick-index offsets in logical track-list coordinates. Convert logical positions when restoring or programmatically navigating.
- Scope the overlay behavior to album detail. Other virtual lists retain their existing independent scrollports.

## Alternatives considered

- Letting the flex header change the list viewport height changes the native thumb ratio and scroll range while the user is scrolling.
- Growing the scrollable content to compensate for the larger viewport would create artificial trailing scroll space and a blank tail after the last track.
- Replacing the native scrollbar with a custom scrollbar would duplicate browser behavior and would not resolve the underlying changing viewport geometry.

## Consequences

The album detail uses a stable scroll viewport and translates between native and logical offsets. The virtual list continues to render only its bounded visible range; the entire album track collection is not mounted in the DOM. Existing list position persistence continues to store the logical track position, independent of cover size.

## Validation

Validated 2026-10-16T00:02:36+08:00 using the fixed `release/moemusicplayer.exe` built from commit `724881bfc6f2eeba0cbf18f2d1c4ea85bd9b4c2e`, with the existing Windows library and WebView2 CDP:

- Album detail for `hanser Cover` (103 tracks) kept `clientHeight=652px`, `scrollHeight=5371px`, and scrollbar ratio `12.1393%` while the cover changed through 148, 138, 108, and 64 CSS px.
- The cover remained collapsed while scrolling upward and returned to 148px only after the logical list reached its top.
- At the bottom, the final row ended at the viewport bottom (757.8px vs. 758px); no artificial blank tail was added.
- `node --test`: 270 passed, 0 failed. `npm run check`: 0 errors and 0 warnings.
