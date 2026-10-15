# ADR 0012: Stable album-detail scrollport during cover collapse

## Status

Accepted — 2026-10-15T00:00:00+08:00

## Context

The album-detail cover shrinks while the track list scrolls. When the heading and list were separate flex siblings, shrinking the heading enlarged the list's `clientHeight` while its full `scrollHeight` remained constant. The native scrollbar therefore changed thumb size during a gesture, and its scroll range changed under the pointer.

## Decision

- Keep one fixed-size native scrollport covering the album-detail viewport. Its scrollable extent remains the full heading reserve, sortable track header, and virtual track content.
- Draw the album heading and sortable track header as overlays above the virtual rows. As the cover collapses, the overlays expose more of the same scrollport; neither its `clientHeight` nor its `scrollHeight` is changed by the collapse.
- Drive the cover size, heading size, sortable-header position, and virtual-row clipping from the same named CSS scroll timeline. Row clipping starts at `collapseRange + rowTop` and completes one row height later, so the mask follows compositor scrolling without waiting for a main-thread `scroll` callback.
- Keep the established JavaScript row-clip calculation as a fallback for WebView runtimes without scroll timelines. The timeline animates by scroll offset, not by elapsed time, and preserves the one-CSS-pixel-per-scroll-pixel mapping.
- Preserve a logical track-list scroll offset separately from the native scroll offset. While the cover is collapsing, one logical pixel advances the native scroll position by two pixels; after the cover reaches 64 CSS px, native and logical movement match. Upward movement keeps the cover size until the logical list returns to its top.
- Keep saved list positions, database paging, grouping, sorting, and quick-index offsets in logical track-list coordinates. Convert logical positions when restoring or programmatically navigating.
- Scope the overlay behavior to album detail. Other virtual lists retain their existing independent scrollports.

## Alternatives considered

- Letting the flex header change the list viewport height changes the native thumb ratio and scroll range while the user is scrolling.
- Recomputing every virtual row's `clip-path` in a main-thread scroll handler can trail native compositor scrolling during rapid scrollbar-thumb drags.
- Growing the scrollable content to compensate for the larger viewport would create artificial trailing scroll space and a blank tail after the last track.
- Replacing the native scrollbar with a custom scrollbar would duplicate browser behavior and would not resolve the underlying changing viewport geometry.

## Consequences

The album detail uses a stable scroll viewport and translates between native and logical offsets. The virtual list continues to render only its bounded visible range; the entire album track collection is not mounted in the DOM. Existing list position persistence continues to store the logical track position, independent of cover size.

## Validation

Validated 2026-10-16T01:19:40+08:00 using the fixed `release/moemusicplayer.exe` built from commit `58e586d64fe82cc33ee3570ed7cf2c7ab4c228f6`, with the existing Windows library and WebView2 CDP:

- Album detail for `hanser Cover` (103 tracks) kept `clientHeight=652px` and `scrollHeight=5371px` while the cover collapsed. The title overlay and sortable grid width matched the viewport's 1269px client width after subtracting the 15px native scrollbar gutter.
- CDP hit testing at the scrollbar gutter over the sortable header returned the scroll viewport. Clicking there moved `scrollTop` from 0 to 820.8px; dragging the thumb moved it from 2753.2px to 3403.6px.
- Rapid wheel scrolling sampled 1,793 rendered-row geometry states; the maximum difference between the row clip and the header boundary was 0.00011 CSS px.
- `node --test`: 274 passed, 0 failed. `npm run check`: 0 errors and 0 warnings.
