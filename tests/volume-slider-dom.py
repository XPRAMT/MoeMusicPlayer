from __future__ import annotations

import json
import subprocess
import urllib.request
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread

from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parents[1]
URL = "http://127.0.0.1:1452/tests/volume-slider-harness.html"
DIST = ROOT / "target" / "volume-slider-dom"


def build_harness() -> None:
    result = subprocess.run(
        [
            "node",
            str(ROOT / "node_modules" / "vite" / "bin" / "vite.js"),
            "build",
            "--config",
            str(ROOT / "vite.volume-slider-harness.config.ts"),
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=120,
    )
    if result.returncode != 0:
        raise RuntimeError(f"Vite volume harness build failed:\n{result.stdout}\n{result.stderr}")


def check_drag(page) -> None:
    page.set_viewport_size({"width": 1440, "height": 900})
    page.goto(URL, wait_until="domcontentloaded", timeout=10_000)
    page.wait_for_selector("input.volume-slider", timeout=10_000)
    slider = page.locator("input.volume-slider")
    page.wait_for_function("() => !document.querySelector('input.volume-slider').disabled")
    page.wait_for_function("() => document.querySelector('[aria-label=播放控制] .play-button')?.getAttribute('aria-label') === '暫停'")
    page.evaluate("""() => {
      const slider = document.querySelector('input.volume-slider');
      window.__volumeHarness.pointerEvents = [];
      for (const name of ['pointerdown', 'pointermove', 'pointerup', 'input', 'change']) {
        slider.addEventListener(name, (event) => window.__volumeHarness.pointerEvents.push({
          type: event.type,
          clientX: event.clientX,
          value: slider.value,
          disabled: slider.disabled,
          target: event.target === slider,
        }), true);
      }
      slider.addEventListener('input', () => {
        window.__volumeHarness.inputCount += 1;
      });
    }""")

    geometry = page.evaluate("""() => {
      const slider = document.querySelector('input.volume-slider');
      const bounds = slider.getBoundingClientRect();
      const fraction = Number(slider.value);
      return {
        left: bounds.left,
        width: bounds.width,
        y: bounds.top + bounds.height / 2,
        fraction,
        paddingLeft: getComputedStyle(slider).paddingLeft,
        paddingRight: getComputedStyle(slider).paddingRight,
        appearance: getComputedStyle(slider).appearance,
        sameElementAfterRender: slider.isConnected,
      };
    }""")
    assert geometry["fraction"] == 0.5, "harness must start with the thumb centered"
    start_x = geometry["left"] + geometry["width"] / 2
    end_x = geometry["left"] + geometry["width"] - 6
    middle_x = start_x + (end_x - start_x) * 0.55
    print("geometry=" + json.dumps(geometry, ensure_ascii=False))
    print(f"drag={start_x:.2f}->{end_x:.2f}")

    page.evaluate("window.__volumeHarness.holdVolumeAcks = true")
    page.mouse.move(start_x, geometry["y"])
    page.mouse.down()
    page.mouse.move(middle_x, geometry["y"], steps=5)
    page.wait_for_function("() => window.__volumeHarness.volumeRequests.length === 1")
    first_command = page.evaluate("""() => ({
      disabled: document.querySelector('input.volume-slider').disabled,
      value: Number(document.querySelector('input.volume-slider').value),
      progressDisabled: document.querySelector('input.progress-slider').disabled,
      inputCount: window.__volumeHarness.inputCount,
      request: window.__volumeHarness.volumeRequests[0],
      inFlight: window.__volumeHarness.isVolumeCommandInFlight(),
    })""")
    print("first-command=" + json.dumps(first_command, ensure_ascii=False))
    assert first_command["inputCount"] >= 4, "pointer drag did not reach multiple volume input positions"
    assert not first_command["disabled"], "volume slider became disabled during its own pointer drag"
    assert not first_command["progressDisabled"], "volume update disabled the independent playback progress control"
    assert first_command["value"] > 0.7, "thumb did not follow the pointer during the drag"
    assert first_command["inFlight"], "the delayed volume ACK was not held in flight"

    page.mouse.move(end_x, geometry["y"], steps=8)
    latest_draft = page.evaluate("""() => ({
      value: Number(document.querySelector('input.volume-slider').value),
      disabled: document.querySelector('input.volume-slider').disabled,
      progressDisabled: document.querySelector('input.progress-slider').disabled,
      requests: window.__volumeHarness.volumeRequests.length,
      inputCount: window.__volumeHarness.inputCount,
      inFlight: window.__volumeHarness.isVolumeCommandInFlight(),
    })""")
    assert latest_draft["value"] > 0.97, "thumb did not reach the dragged endpoint while the ACK was pending"
    assert not latest_draft["disabled"], "pending volume ACK disabled the volume slider"
    assert not latest_draft["progressDisabled"], "pending volume ACK disabled playback progress"
    assert latest_draft["requests"] == 1, "drag generated concurrent volume requests before release"
    assert latest_draft["inputCount"] >= 4, "pointer drag did not generate multiple input values"
    page.mouse.up()

    held_ack = page.evaluate("""() => ({
      requests: window.__volumeHarness.volumeRequests.slice(),
      pendingAcks: window.__volumeHarness.pendingVolumeAcks.length,
      draft: Number(document.querySelector('input.volume-slider').value),
    })""")
    assert held_ack["requests"] == [first_command["request"]], "pending updates were not coalesced behind one in-flight command"
    assert held_ack["pendingAcks"] == 1, "expected exactly one held volume ACK"
    assert held_ack["draft"] > 0.97, "pointer release regressed the latest local volume draft"
    page.evaluate("window.__volumeHarness.releaseVolumeAck()")
    page.wait_for_function("() => window.__volumeHarness.volumeRequests.length === 2")
    after_first_ack = page.evaluate("""() => ({
      requests: window.__volumeHarness.volumeRequests.slice(),
      sliderValue: Number(document.querySelector('input.volume-slider').value),
      inFlight: window.__volumeHarness.isVolumeCommandInFlight(),
      pendingAcks: window.__volumeHarness.pendingVolumeAcks.length,
    })""")
    assert abs(after_first_ack["requests"][1] - 1) < 0.03, "final flush did not send the latest endpoint after the prior ACK"
    assert after_first_ack["inFlight"], "latest value command was not serialized after the first ACK"
    assert after_first_ack["pendingAcks"] == 1, "expected the final value ACK to be held"
    assert after_first_ack["sliderValue"] > 0.97, "the earlier ACK regressed the newer draft value"
    page.evaluate("window.__volumeHarness.releaseVolumeAck()")
    page.wait_for_function("() => !window.__volumeHarness.isVolumeCommandInFlight()")

    final_state = page.evaluate("""() => ({
      requests: window.__volumeHarness.volumeRequests,
      inputCount: window.__volumeHarness.inputCount,
      maxConcurrent: window.__volumeHarness.maxConcurrentVolumeCommands,
      volume: window.__volumeHarness.snapshot().volume,
      sliderValue: Number(document.querySelector('input.volume-slider').value),
      progressValue: Number(document.querySelector('input.progress-slider').value),
      progressDisabled: document.querySelector('input.progress-slider').disabled,
      playbackLabel: document.querySelector('.play-button').getAttribute('aria-label'),
      pointerEvents: window.__volumeHarness.pointerEvents,
    })""")
    assert final_state["inputCount"] >= 4, "volume input events did not track multiple pointer positions"
    assert final_state["inputCount"] > len(final_state["requests"]), "volume input events were not coalesced"
    assert len(final_state["requests"]) == 2, "volume drag must use one active request plus one latest-value flush"
    assert final_state["maxConcurrent"] == 1, "volume commands were sent concurrently"
    assert abs(final_state["sliderValue"] - 1) < 0.03, "slider did not finish at the dragged endpoint"
    assert abs(final_state["volume"] - 1) < 0.03, "backend did not finish at the dragged endpoint"
    assert final_state["progressDisabled"] is False, "volume updates left progress control disabled"
    assert final_state["playbackLabel"] == "暫停", "volume updates changed the playing state"
    distinct_moves = {event["clientX"] for event in final_state["pointerEvents"] if event["type"] == "pointermove"}
    assert len(distinct_moves) >= 5, "the test did not exercise multiple distinct pointer positions"
    print("drag-result=" + json.dumps({
        "requests": final_state["requests"],
        "inputCount": final_state["inputCount"],
        "distinctPointerPositions": len(distinct_moves),
        "maxConcurrent": final_state["maxConcurrent"],
        "volume": final_state["volume"],
        "sliderValue": final_state["sliderValue"],
        "progressValue": final_state["progressValue"],
        "playbackLabel": final_state["playbackLabel"],
    }, ensure_ascii=False))

    page.evaluate("window.__volumeHarness.holdVolumeAcks = false")
    slider.focus()
    slider.press("ArrowLeft")
    page.wait_for_function("() => Math.abs(window.__volumeHarness.snapshot().volume - 0.99) < 0.005")
    keyboard_state = page.evaluate("""() => ({
      volume: window.__volumeHarness.snapshot().volume,
      sliderValue: Number(document.querySelector('input.volume-slider').value),
      progressValue: Number(document.querySelector('input.progress-slider').value),
      playbackLabel: document.querySelector('.play-button').getAttribute('aria-label'),
      maxConcurrent: window.__volumeHarness.maxConcurrentVolumeCommands,
    })""")
    assert abs(keyboard_state["volume"] - 0.99) < 0.03, "keyboard change did not flush the adjusted volume"
    assert abs(keyboard_state["sliderValue"] - 0.99) < 0.03, "keyboard change did not update the volume slider"
    assert keyboard_state["progressValue"] == 12_000, "volume change altered playback position"
    assert keyboard_state["playbackLabel"] == "暫停", "volume change altered playback state"
    assert keyboard_state["maxConcurrent"] == 1, "keyboard volume command overlapped another command"


def main() -> None:
    build_harness()
    server = ThreadingHTTPServer(
        ("127.0.0.1", 1452),
        partial(SimpleHTTPRequestHandler, directory=str(DIST)),
    )
    server_thread = Thread(target=server.serve_forever, daemon=True)
    server_thread.start()
    try:
        with urllib.request.urlopen(URL, timeout=3):
            pass
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True)
            page = browser.new_page()
            page.set_default_timeout(5_000)
            page.set_default_navigation_timeout(10_000)
            errors: list[str] = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.on("console", lambda message: errors.append(message.text) if message.type == "error" else None)
            check_drag(page)
            assert not errors, "browser errors: " + "; ".join(errors)
            browser.close()
        print("Volume slider DOM checks passed.")
    finally:
        server.shutdown()
        server.server_close()
        server_thread.join(timeout=5)


if __name__ == "__main__":
    main()
