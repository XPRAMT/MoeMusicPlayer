# Lyric line changes lead the line start

Status: Accepted and implemented on 2026-10-22

Decision date: 2026-10-22, Asia/Taipei (UTC+08:00)

## Context

On the Now Playing page a new line only started its change animation (smooth scroll, 140 ms colour/opacity
transitions) when the line started, so the page settled after the singing had already begun and the lyrics felt late.
The smooth scroll's length was chosen by the browser, so it could not be made to end at a given moment.

## Decision

- One constant, `LYRIC_CUE_LEAD_MS` = 300 ms in `src/lib/lyrics-cue-timing.ts`, is both the lead and the animation
  length; CSS reads it through `--lyric-cue-ms`. Not user-adjustable.
- While playing, a line is shown `lead` before its start: lead = min(300 ms, gap to the previous line); the first line
  is never shown before position 0. The user's lyric delay still applies on top. The animation runs for the time left
  until the line really starts, so a shortened lead or resuming inside the lead animates for what remains.
- The scroll is a fixed-length ease-out rAF tween (only while it runs) instead of `scrollTo({ behavior: 'smooth' })`.
- `activeTimedIndex` stays the line really playing; `displayTimedIndex` drives what is drawn. Paused playback does not
  lead; seeks while playing, track changes and reduced motion stay instant. Play counts and the previous-button
  three-second rule use the real position.
- Taskbar lyrics: no fade; the line switches at its real start (no lead); long-line scrolling starts at the real start.

## Consequences

- The highlighted line on the Now Playing page changes up to 300 ms before the line's timestamp, by design.
- Playback-clock corrections can still move a change by up to the clock's convergence error (< 250 ms, ADR 0028).
