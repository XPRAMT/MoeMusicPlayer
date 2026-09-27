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

## Constraints

- Opening files, decoding, output-device work, and seeking stay off the caller's
  thread.
- Public control methods enqueue into a bounded queue without waiting; a full or
  stopped worker is reported to the caller.
- Dropping the final player handle sends shutdown and joins the worker.
- The current source is an OS `PathBuf`; converting it to a display string is
  not part of source identity.
- The initial Windows backend uses rodio and CPAL/WASAPI. `AudioBackend` keeps
  the worker boundary replaceable.
