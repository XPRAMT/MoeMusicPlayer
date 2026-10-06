# Windows Audio Domain Context

## Responsibility

This crate plays one local Windows file at a time. It owns native output,
decoding, playback position, seeking, and volume. It does not own the play queue,
track identity, metadata, or renderer state.

## Terms

- **Player handle:** A cloneable command and observation surface for one worker.
- **Backend:** A synchronous audio implementation used only by that worker.
- **Playback snapshot:** The latest state, position, duration, volume, and error.
- **Playback event:** A bounded state-change or error notification. The snapshot
  remains authoritative if an event is dropped because the event queue is full.
- **System Media Transport Controls (SMTC) session:** Windows' media controls
  and now-playing display associated with this app's top-level window.
- **Media-control capability:** A declaration that the current playback state
  or queue can honor a command; unsupported commands stay disabled.
- **Now-playing metadata:** The track title, artist, and album shown in system
  media surfaces, separate from the track's stable app identity.
- **Resampling mode:** How a track whose sample rate differs from the output is
  converted. *High quality* (default) keeps the stream at the endpoint's
  shared-mode mix rate and converts in-app; *Windows built-in* opens the stream
  at the track's rate and lets the Windows audio engine convert. Both are
  shared mode and not bit-perfect.
- **Resampling info:** The snapshot's view of the current sample-rate path:
  mode, track rate, stream rate, endpoint mix rate, who converts, and why a
  Windows built-in open fell back to in-app conversion.

## Constraints

- Opening files, decoding, output-device work, and seeking stay off the caller's
  thread.
- Public control methods enqueue into a bounded queue without waiting; a full or
  stopped worker is reported to the caller.
- The current source is an OS `PathBuf`; converting it to a display string is
  not part of source identity.
- The initial Windows backend uses rodio and CPAL/WASAPI. `AudioBackend` keeps
  the worker boundary replaceable.
- Backend methods never wait on CPAL's render callback. Output loss (stream
  error, stalled heartbeat) or a default-device change is reported through
  `OutputStatus`; the worker rebuilds the backend via a recoverable factory and
  restores track, position, volume, and play/pause intent. Without a device the
  snapshot carries a recoverable `OutputDevice` error and the worker retries.
- Dropping the final handle waits at most a few seconds for the worker and
  detaches it if a backend call is stuck.
- In-app resampling (`resample.rs`, rubato FFT, 2048-frame chunks, one
  sub-chunk) is built on the worker when a track is attached; it trims the
  filter delay, flushes the tail, resets on seek, and only allocates in the
  render callback when a stream changes rate mid-way. Equal rates bypass it.
- Windows built-in mode reopens the stream only when the track rate changes.
  A refused or mismatched open falls back to the mix rate with in-app
  conversion and a logged warning; failing every open is an output error
  handled by recovery. A rebuilt output reopens in the current mode, directly
  at the last track's rate, before the track is restored.
- Switching modes is an acknowledged worker command; when the output must be
  replaced the track restarts at its current position with its play/pause
  state and volume.
