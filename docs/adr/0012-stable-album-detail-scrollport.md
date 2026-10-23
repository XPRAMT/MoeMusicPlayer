# ADR 0012: Stable album-detail scrollport during cover collapse

## Status

Accepted — 2026-10-15T00:00:00+08:00. Revised 2026-10-16T05:00:00+08:00 (scroller starts below the collapsed heading). Revised 2026-10-23 (cover collapse is a pure compositor transform).

## Context

The album-detail cover shrinks while the track list scrolls. When the heading and list were separate flex siblings, shrinking the heading enlarged the list's `clientHeight` while its `scrollHeight` stayed constant, so the native scrollbar thumb changed size under the pointer.

The first fix kept one full-height scrollport and drew the transparent heading and sortable header as overlays above the rows. Rows under the overlays therefore had to be clipped individually (`clip-path` per row, first in a scroll handler, later through a CSS scroll timeline), and native scroll offsets were mapped to logical list offsets with a custom wheel handler. Per-row clipping trails compositor scrolling: `top`, `width` and `height` animations and newly mounted rows are main-thread work, so fast scrollbar-thumb drags could still show rows above the header.

## Decision

- The track scroller starts at the fixed line `Y0 = headingHeight − collapseRange + tableHeaderHeight`, i.e. below the fully collapsed heading and the sortable header, and extends to the bottom of the album detail. Its position, `clientHeight`, `scrollHeight` and thumb ratio never change while scrolling.
- The scroll content begins with a spacer equal to `collapseRange` (desktop 120 CSS px: 200→80 cover; narrow 80 CSS px: 160→80 cover), followed by the virtual rows. The expanded heading and the sortable header overlay only that spacer, so no row is ever covered and no row clipping is needed.
- The collapse offset is a pure function of the native offset: `collapse = min(scrollTop, collapseRange)` and `listScrollTop = max(0, scrollTop − collapseRange)`. Scrolling down collapses the cover first; scrolling up returns the list to its top before the cover expands.
- The cover is always laid out at its base size (200px desktop, 160px narrow) and the heading keeps its base height; scrolling never changes layout. Collapse is a pure compositor transform driven by the list's native scroll timeline: the cover uses transform: scale() from its top-left corner, the summary and the sortable header use translate(). A separate paint-only border-radius animation (10px ÷ scale) keeps the visual corner radius at 10px. Runtimes without scroll timelines write --album-detail-collapse-progress (0–1) from the scroll event instead; with scroll timelines no per-scroll inline style is written.
- Wheel, keyboard and scrollbar input all use native browser scrolling. There is no wheel interception, native/logical remapping or per-row clip.
- Saved list positions, paging, grouping, sorting and quick-index offsets stay in logical track-list coordinates; a positive logical position restores as `listScrollTop + collapseRange`.
- Scope the overlay behavior to album detail. Other virtual lists retain their existing independent scrollports.

## Alternatives considered

- Letting the flex header change the list viewport height changes the thumb ratio and scroll range during a gesture.
- A full-height scrollport with overlays requires per-row clipping that trails compositor scrolling during fast thumb drags (the previous design).
- A static `mask-image` on the full-height scrollport would hide rows behind the header without per-row work, but keeps the native/logical mapping and wheel interception.
- Replacing the native scrollbar with a custom scrollbar would duplicate browser behavior.

## Consequences

The scrollbar track begins at `Y0` instead of the top of the heading. Virtual rows may briefly be blank during very fast thumb drags because rendering and page loading run after compositor scrolling; mitigation (larger overscan, placeholder rows) is a future optimization.

The summary text keeps its expanded width while collapsing (no extra 120px), so long artist names wrap more than before. The cover is scaled down from a 200px layer (source thumbnail 512px) and may look slightly soft at 80px.

## Validation

Validated 2026-10-16T05:10:00+08:00 using `release/moemusicplayer.exe` built from commit `a1e3d41`, through WebView2 CDP on port 9222, on `hanser Cover` (103 tracks):

- Fast scrollbar-thumb drag (three full passes, 1,565 animation-frame samples): no visible row pixel above the sortable header's bottom edge; `clientHeight=515px`, `scrollHeight=5234px` and the scroller top stayed constant; `scrollTop` covered 0–4718.8 px.
- While collapsing, the first row's top equaled the header's bottom edge in every sample; wheel input (verified at the earlier 148→64 px sizing) collapsed the cover before moving rows and restored it only after the list returned to its top.
- `node --test` on `tests/`: all passed. `npm run check`: 0 errors and 0 warnings.

2026-10-23 commit b67f09e: node 528 pass, svelte-check 0 errors; not yet verified in real WebView2 with continuous lyrics.
