<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import PlaybackProgress from '../src/lib/PlaybackProgress.svelte';
  import { effectivePlaybackDurationMs } from '../src/lib/playback-duration';

  const playlistScenario = new URLSearchParams(window.location.search).get('scenario') === 'playlist';
  const trackDurationMs = playlistScenario ? 184_000 : 60_000;
  const audioDurationMs = playlistScenario ? null : 60_000;
  const durationMs = effectivePlaybackDurationMs(audioDurationMs, trackDurationMs);
  let audioPositionMs = $state(5_000);
  let lastSeekRequestMs = $state<number | null>(null);
  let seekRequestCount = $state(0);
  let seekError = $state<string | null>(null);
  let failNextSeek = false;
  let trackId = $state(playlistScenario ? 'track-playlist' : 'track-library');
  let timer: ReturnType<typeof setInterval>;

  onMount(() => {
    timer = setInterval(() => { audioPositionMs += 250; }, 80);
  });

  onDestroy(() => clearInterval(timer));

  function requestSeek(positionMs: number): boolean {
    seekRequestCount += 1;
    if (failNextSeek) {
      failNextSeek = false;
      seekError = 'seek unsupported';
      return false;
    }
    lastSeekRequestMs = positionMs;
    seekError = null;
    window.setTimeout(() => {
      audioPositionMs = positionMs;
    }, 150);
    return true;
  }

  (window as unknown as { setTrack: (nextTrackId: string, nextPositionMs: number) => void }).setTrack =
    (nextTrackId, nextPositionMs) => {
      trackId = nextTrackId;
      audioPositionMs = nextPositionMs;
    };
  (window as unknown as { failNextSeek: () => void }).failNextSeek = () => { failNextSeek = true; };
</script>

<main>
  <output data-testid="audio-snapshot-position">{audioPositionMs}</output>
  <output data-testid="seek-request">{lastSeekRequestMs ?? ''}</output>
  <output data-testid="seek-request-count">{seekRequestCount}</output>
  <output data-testid="seek-error">{seekError ?? ''}</output>
  <PlaybackProgress
    positionMs={audioPositionMs}
    durationMs={durationMs}
    trackId={trackId}
    canControl={true}
    isSending={false}
    onSeek={requestSeek}
  />
</main>
