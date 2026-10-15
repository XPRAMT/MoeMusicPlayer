import type { Readable } from 'svelte/store';

export type TrackTextOverflowMode = 'middle' | 'end';

export const TRACK_TEXT_OVERFLOW_CONTEXT = 'moemusicplayer:track-text-overflow-mode';
export type TrackTextOverflowStore = Readable<TrackTextOverflowMode>;
