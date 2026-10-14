# ADR 0011: ICU4X locale-aware natural collation

## Status

Accepted

## Context

Track title, artist, and album ordering had diverged across SQLite, the artist and album indexes, and browser-side locale sorting. The browser used a fixed Traditional Chinese locale, while database comparisons used a byte-oriented natural comparator. Windows and Android users need the same application rules while following their own locale, and digit runs must always sort by numeric value.

## Decision

- Use ICU4X `Collator` in `player-db` for track, playlist, artist-index, and album-index text ordering. Preserve ICU's locale defaults for case and accent sensitivity; always set numeric ordering.
- On Windows, call `GetUserDefaultLocaleName`. Translate a Windows sorting suffix such as `_pronun` into the corresponding ICU collation type when present.
- On Android, read `Locale.getDefault()` through the Android JVM. When its country is `TW`, explicitly select ICU Zhuyin collation.
- Use the same registered SQLite collation for title, artist, and album. Use a typed IPC command backed by that collator for frontend-generated artist and album index rows and artist names displayed in album details.
- Persist the locale, collation type, ICU collator crate version, and compiled collation-data version in SQLite. Rebuild the title index when this signature changes. Checked startup validates with the database's stored collation before writable migration.
- Keep the existing natural filename sort for the Windows File Explorer sibling queue; this request covers metadata text ordering, not path traversal order.

## Alternatives considered

- A browser-only `Intl.Collator` cannot guarantee identical Windows and Android data, locale preferences, or matching SQLite page and queue order.
- The existing `NATURAL_NOCASE` comparator ignores locale collation for non-ASCII text and case/accent preferences.
- Platform-specific ordering implementations would produce different results across Windows and Android.

## Consequences

ICU4X compiled data adds a small dependency/data footprint but provides a single cross-platform comparator. Locale or ICU data changes cause an indexed title rebuild. Artist and album index results now require one backend sort-index request after their rows are assembled. Existing natural filename ordering for the external-file queue is unchanged.
