<script lang="ts">
  import { formatDuration } from './format';
  import {
    clampPlaybackPosition,
    isPlaybackSeekableDuration,
    playbackSeekDisplayPosition,
    shouldReleasePlaybackSeekDraft,
  } from './playback-scrubber';
  import type { TimelineStyle } from './ipc';

  let {
    positionMs = 0,
    durationMs = null,
    trackId = null,
    canControl = false,
    isSending = false,
    timelineStyle = 'edge',
    onSeek,
  }: {
    positionMs: number;
    durationMs: number | null;
    trackId: string | null;
    canControl: boolean;
    isSending: boolean;
    timelineStyle?: TimelineStyle;
    onSeek: (positionMs: number) => boolean | void | Promise<boolean | void>;
  } = $props();

  let draftPositionMs = $state<number | null>(null);
  let draftTrackId = $state<string | null>(null);
  let seekBaselineMs = $state<number | null>(null);
  let seekSettled = $state(false);
  let pointerActive = false;
  let seekInFlight = false;

  const displayedPositionMs = $derived(
    playbackSeekDisplayPosition(
      positionMs,
      durationMs,
      draftTrackId === trackId ? draftPositionMs : null,
    ),
  );
  const progressPercent = $derived(
    durationMs != null && durationMs > 0
      ? Math.min(100, Math.max(0, (displayedPositionMs / durationMs) * 100))
      : 0,
  );
  const seekEnabled = $derived(canControl && trackId !== null && isPlaybackSeekableDuration(durationMs));
  // Keep the range input enabled while non-seek commands are busy so :disabled opacity does not flash the timeline.

  function discardSeekDraft(): void {
    pointerActive = false;
    draftPositionMs = null;
    draftTrackId = null;
    seekBaselineMs = null;
    seekSettled = false;
  }

  $effect(() => {
    if (draftTrackId !== null && draftTrackId !== trackId) {
      discardSeekDraft();
      return;
    }
    if (shouldReleasePlaybackSeekDraft(positionMs, draftPositionMs, pointerActive, seekSettled, seekBaselineMs)) {
      draftPositionMs = null;
      draftTrackId = null;
      seekBaselineMs = null;
      seekSettled = false;
    }
  });

  function beginPointerSeek(): void {
    if (!seekEnabled || isSending) return;
    pointerActive = true;
    draftTrackId = trackId;
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
      discardSeekDraft();
      return;
    }
    const requestedPositionMs = draftPositionMs;
    pointerActive = false;
    if (!seekEnabled || requestedPositionMs === positionMs) {
      discardSeekDraft();
      return;
    }
    if (isSending || seekInFlight) return;
    seekBaselineMs = positionMs;
    seekSettled = false;
    seekInFlight = true;
    void Promise.resolve(onSeek(requestedPositionMs)).then((accepted) => {
      seekInFlight = false;
      if (pointerActive || draftPositionMs !== requestedPositionMs) return;
      if (accepted === false) {
        discardSeekDraft();
        return;
      }
      seekSettled = true;
    });
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
  <span class="progress-time progress-time-elapsed">{formatDuration(displayedPositionMs)}</span>
  <div class="progress-hit">
    <input
      class="progress-slider"
      type="range"
      min="0"
      max={Math.max(1, durationMs ?? 0)}
      value={displayedPositionMs}
      style={`--progress-pct: ${progressPercent}%`}
      aria-label="播放進度"
      disabled={!seekEnabled}
      onpointerdown={beginPointerSeek}
      oninput={updateDraft}
      onchange={commitDraft}
      onblur={commitDraft}
    />
  </div>
  <span class="progress-time progress-time-duration">{formatDuration(durationMs)}</span>
</div>
