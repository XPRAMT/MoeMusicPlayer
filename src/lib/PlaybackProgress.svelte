<script lang="ts">
  import { untrack } from 'svelte';
  import { tr } from './i18n.js';
  import { formatDuration } from './format';
  import {
    clampPlaybackPosition,
    isPlaybackSeekableDuration,
    playbackSeekDisplayPosition,
    shouldReleasePlaybackSeekDraft,
  } from './playback-scrubber';
  import { playbackSeekKeyStepMs } from './shortcuts.js';
  import { progressRun, type ProgressMotion } from './progress-run.js';
  import { bufferingCueVisible } from './buffering-cue.js';
  import type { TimelineStyle } from './ipc';
  import {
    estimatePlaybackPosition,
    msUntilPlaybackPosition,
    playbackProgressRun,
    type PlaybackClock,
  } from './playback-clock';

  let {
    positionMs = 0,
    durationMs = null,
    trackId = null,
    canControl = false,
    isSending = false,
    timelineStyle = 'edge',
    clock = null,
    playbackState = null,
    onSeek,
  }: {
    positionMs: number;
    durationMs: number | null;
    trackId: string | null;
    canControl: boolean;
    isSending: boolean;
    timelineStyle?: TimelineStyle;
    /** Playback clock (ADR 0028). Without it the bar shows `positionMs` as is. */
    clock?: PlaybackClock | null;
    /** Snapshot state; `buffering` pulses the thumb (the only buffering cue). */
    playbackState?: string | null;
    onSeek: (positionMs: number) => boolean | void | Promise<boolean | void>;
  } = $props();

  let draftPositionMs = $state<number | null>(null);
  let draftTrackId = $state<string | null>(null);
  let seekBaselineMs = $state<number | null>(null);
  let seekSettled = $state(false);
  let pointerActive = false;
  let seekInFlight = false;

  // The elapsed time text is re-read once per whole second (secondTick); the bar itself runs as a CSS animation.
  let secondTick = $state(0);
  const clockPositionMs = $derived.by(() => {
    secondTick;
    return clock ? estimatePlaybackPosition(clock, performance.now()) : positionMs;
  });
  $effect(() => {
    secondTick;
    const current = clock;
    if (!current?.playing) return;
    const now = performance.now();
    const wait = msUntilPlaybackPosition(current, now, (Math.floor(estimatePlaybackPosition(current, now) / 1000) + 1) * 1000);
    if (wait === null) return;
    const timer = setTimeout(() => { secondTick += 1; }, wait + 5);
    return () => clearTimeout(timer);
  });
  // Buffering pulse: on while the state is `buffering`, off the moment it changes. The pulse itself is a CSS animation
  // on `.progress-thumb` (app.css).
  const bufferingCue = $derived(bufferingCueVisible(playbackState));
  const hasDraft = $derived(draftTrackId === trackId && draftPositionMs !== null);
  const displayedPositionMs = $derived(
    playbackSeekDisplayPosition(
      clockPositionMs,
      durationMs,
      draftTrackId === trackId ? draftPositionMs : null,
    ),
  );
  const progressPercent = $derived(
    durationMs != null && durationMs > 0
      ? Math.min(100, Math.max(0, (displayedPositionMs / durationMs) * 100))
      : 0,
  );
  // Compositor run of the bar (progress-run.js): from the current ratio to 1 in the remaining time, linear, transform
  // only, so it costs no style, layout or paint work per frame. It restarts only when the clock jumps (seek, new track,
  // play/pause, deviation > 250 ms), a stall freezes or resumes the clock (`runKey`, parked while frozen) or a drag
  // ends; smooth corrections leave it running. `run.name` alternates so that a
  // restart at identical numbers is still a change. Linear and sub-pixel (the user chose smoothness over snapping).
  let run = $state<{ ratio: number; remainingMs: number | null; name: string }>({ ratio: 0, remainingMs: null, name: 'playback-progress-run-a' });
  $effect(() => {
    const current = clock;
    current?.jumpId;
    current?.playing;
    // A stall freezes the clock (ADR 0028): `runKey` parks the run at the frozen ratio and restarts it on resume.
    current?.runKey;
    if (hasDraft) return;
    const next = playbackProgressRun(current, performance.now());
    const name = untrack(() => run.name) === 'playback-progress-run-a' ? 'playback-progress-run-b' : 'playback-progress-run-a';
    run = { ...next, name };
  });
  const progressMotion = $derived.by((): ProgressMotion => {
    if (hasDraft || !clock || durationMs == null || durationMs <= 0 || durationMs !== clock.durationMs) {
      return { ratio: progressPercent / 100, remainingMs: null, key: 'parked' };
    }
    return run.remainingMs === null
      ? { ratio: run.ratio, remainingMs: null, key: 'parked' }
      : { ratio: run.ratio, remainingMs: Math.round(run.remainingMs), key: run.name };
  });
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

  // Arrow / Page keys on the focused bar: the native range step is 1 ms and starts from the once-a-second text
  // position, so it would seek slightly backwards. Seek from the live clock instead (bound shortcuts run first and
  // already called preventDefault). Home/End stay native (absolute).
  function handleSeekKeydown(event: KeyboardEvent): void {
    if (event.defaultPrevented) return;
    const step = playbackSeekKeyStepMs(event.key);
    if (step === null) return;
    event.preventDefault();
    if (event.repeat || !seekEnabled || isSending || seekInFlight) return;
    const now = clock ? estimatePlaybackPosition(clock, performance.now()) : positionMs;
    void onSeek(clampPlaybackPosition(now + step, durationMs));
  }

  function finishPointerSeek(): void {
    if (pointerActive) commitDraft();
  }
</script>

<svelte:window onpointerup={finishPointerSeek} onpointercancel={cancelDraft} />

<div class="progress-row" data-timeline-style={timelineStyle} data-buffering={bufferingCue ? 'true' : undefined}>
  <span class="progress-time progress-time-elapsed">{formatDuration(displayedPositionMs)}</span>
  <div class="progress-hit" use:progressRun={progressMotion}>
    <div class="progress-track" aria-hidden="true"><div class="progress-band"></div></div>
    <input
      class="progress-slider"
      type="range"
      min="0"
      max={Math.max(1, durationMs ?? 0)}
      value={displayedPositionMs}
      aria-label={tr("播放進度")}
      disabled={!seekEnabled}
      onpointerdown={beginPointerSeek}
      onkeydown={handleSeekKeydown}
      oninput={updateDraft}
      onchange={commitDraft}
      onblur={commitDraft}
    />
    <div class="progress-thumb-rail" aria-hidden="true"><div class="progress-thumb-mover"><span class="progress-thumb"></span></div></div>
  </div>
  <span class="progress-time progress-time-duration">{formatDuration(durationMs)}</span>
</div>
