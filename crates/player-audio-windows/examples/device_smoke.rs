use std::env;
use std::error::Error;
use std::thread;
use std::time::{Duration, Instant};

use player_audio_windows::{AudioError, PlaybackState, PlayerHandle};

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os().nth(1).ok_or(
        "usage: cargo run -p player-audio-windows --example device_smoke -- <local-audio-file>",
    )?;
    let player = PlayerHandle::new()?;

    player.load(path)?;
    wait_for(&player, PlaybackState::Ready, Duration::from_secs(10))?;
    let duration = player.snapshot().duration;
    println!("loaded: duration={duration:?}");

    player.play()?;
    wait_for(&player, PlaybackState::Playing, Duration::from_secs(2))?;
    wait_until_position(&player, Duration::from_millis(350), Duration::from_secs(3))?;
    let before_pause = player.snapshot().position;

    player.pause()?;
    wait_for(&player, PlaybackState::Paused, Duration::from_secs(2))?;
    thread::sleep(Duration::from_millis(250));
    let after_pause = player.snapshot().position;
    if after_pause > before_pause + Duration::from_millis(80) {
        return Err(
            format!("position advanced while paused: {before_pause:?} -> {after_pause:?}").into(),
        );
    }

    let seek_target = duration
        .map(|duration| duration / 2)
        .unwrap_or(Duration::from_secs(1));
    player.seek(seek_target)?;
    wait_until_near(
        &player,
        seek_target,
        Duration::from_millis(200),
        Duration::from_secs(3),
    )?;
    player.set_volume(0.35)?;
    wait_until_volume(&player, 0.35, Duration::from_secs(2))?;

    player.play()?;
    wait_for(&player, PlaybackState::Playing, Duration::from_secs(2))?;
    player.stop()?;
    wait_for(&player, PlaybackState::Stopped, Duration::from_secs(2))?;
    println!("device smoke passed: pause stable, seek reached, volume set, stop reset");
    Ok(())
}

fn wait_for(
    player: &PlayerHandle,
    state: PlaybackState,
    timeout: Duration,
) -> Result<(), AudioError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let snapshot = player.snapshot();
        if snapshot.state == state {
            return Ok(());
        }
        if let Some(error) = snapshot.last_error {
            return Err(error);
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(AudioError::Backend(format!(
        "timed out waiting for {state:?}"
    )))
}

fn wait_until_position(
    player: &PlayerHandle,
    minimum: Duration,
    timeout: Duration,
) -> Result<(), AudioError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let snapshot = player.snapshot();
        if snapshot.position >= minimum {
            return Ok(());
        }
        if let Some(error) = snapshot.last_error {
            return Err(error);
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(AudioError::Backend(format!(
        "playback position did not reach {minimum:?}"
    )))
}

fn wait_until_near(
    player: &PlayerHandle,
    target: Duration,
    tolerance: Duration,
    timeout: Duration,
) -> Result<(), AudioError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let position = player.snapshot().position;
        if position.abs_diff(target) <= tolerance {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(AudioError::Backend(format!(
        "seek did not reach {target:?}"
    )))
}

fn wait_until_volume(
    player: &PlayerHandle,
    volume: f32,
    timeout: Duration,
) -> Result<(), AudioError> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if (player.snapshot().volume - volume).abs() < f32::EPSILON {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(AudioError::Backend(
        "volume update was not reflected in snapshot".to_owned(),
    ))
}
