import { invokeCommand } from './ipc';
import type { LyricsTrackResult, TrackLyrics } from './ipc';

/** The backend calls the lyrics controller needs (the controller is created once, by the app shell). */
export type LyricsApi = {
  getTrack: (args: { trackId: string }) => Promise<LyricsTrackResult>;
  search: (args: { trackId: string; requestId: string; manual?: boolean; query?: string; queryAlternates?: string[] }) => Promise<LyricsTrackResult>;
  selectCandidate: (args: { trackId: string; candidateId: string }) => Promise<TrackLyrics>;
  cancelSearch: (args: { requestId: string }) => Promise<void>;
  clearTrack: (args: { trackId: string }) => Promise<LyricsTrackResult>;
  loadMore?: (args: { trackId: string }) => Promise<LyricsTrackResult>;
};

export const defaultLyricsApi: LyricsApi & Required<Pick<LyricsApi, 'loadMore'>> = {
  getTrack: (args) => invokeCommand('lyrics_get_track', args),
  search: (args) => invokeCommand('lyrics_search', args),
  selectCandidate: (args) => invokeCommand('lyrics_select_candidate', args),
  cancelSearch: (args) => invokeCommand('lyrics_cancel_search', args),
  clearTrack: (args) => invokeCommand('lyrics_clear_track', args),
  loadMore: (args) => invokeCommand('lyrics_load_more', args),
};
