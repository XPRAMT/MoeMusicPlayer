<script lang="ts">
  import { formatDuration } from './format';
  import {
    clampPlaybackPosition,
    isPlaybackSeekableDuration,
    playbackSeekDisplayPosition,
  } from './playback-scrubber';
  import type { TimelineStyle } from './ipc';

  let {
    positionMs = 0,
    durationMs = null,
    trackId = null,
    canControl = false,
    isSending = false,
    timelineStyle = 'line',
    onSeek,
  }: {
    positionMs: number;
    durationMs: number | null;
    trackId: string | null;
    canControl: boolean;
    isSending: boolean;
    timelineStyle?: TimelineStyle;
    onSeek: (positionMs: number) => void | Promise<void>;
  } = $props();

  let draftPositionMs = $state<number | null>(null);
  let draftTrackId = $state<string | null>(null);
  let pointerActive = false;

  const displayedPositionMs = $derived(
    playbackSeekDisplayPosition(
      positionMs,
      durationMs,
      draftTrackId === trackId ? draftPositionMs : null,
    ),
  );
  const seekEnabled = $derived(canControl && trackId !== null && isPlaybackSeekableDuration(durationMs));
  // Keep the range input enabled while non-seek commands are busy so :disabled opacity does not flash the timeline.

  $effect(() => {
    if (draftTrackId !== null && draftTrackId !== trackId) cancelDraft();
  });

  function beginPointerSeek(): void {
    if (!seekEnabled || isSending) return;
    pointerActive = true;
    draftTrackId = trackId;
    draftPositionMs = displayedPositionMs;
  }

  function updateDraft(event: Event): void {
    if (!seekEnabled || isSending) return;
    const value = Number((event.currentTarget as HTMLInputElement).value);
    if (!pointerActive) {
      draftTrackId = trackId;
      draftPositionMs = displayedPositionMs;
    }
    draftPositionMs = clampPlaybackPosition(value, durationMs);
  }

  function commitDraft(): void {
    if (draftPositionMs === null) return;
    if (draftTrackId !== trackId) {
      cancelDraft();
      return;
    }
    const requestedPositionMs = draftPositionMs;
    draftPositionMs = null;
    draftTrackId = null;
    pointerActive = false;
    if (!seekEnabled || isSending || requestedPositionMs === positionMs) return;
    void onSeek(requestedPositionMs);
  }

  function cancelDraft(): void {
    if (!pointerActive) return;
    pointerActive = false;
    draftPositionMs = null;
    draftTrackId = null;
  }

  function finishPointerSeek(): void {
    if (pointerActive) commitDraft();
  }
</script>

<svelte:window onpointerup={finishPointerSeek} onpointercancel={cancelDraft} />

<div class="progress-row" data-timeline-style={timelineStyle}>
  <span>{formatDuration(displayedPositionMs)}</span>
  <input
    class="progress-slider"
    type="range"
    min="0"
    max={Math.max(1, durationMs ?? 0)}
    value={displayedPositionMs}
    aria-label="播放進度"
    disabled={!seekEnabled}
    onpointerdown={beginPointerSeek}
    oninput={updateDraft}
    onchange={commitDraft}
    onblur={commitDraft}
  />
  <span>{formatDuration(durationMs)}</span>
</div>
