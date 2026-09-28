from __future__ import annotations

import subprocess
import urllib.request
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread

from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parents[1]
URL = "http://127.0.0.1:1451/tests/playback-progress-harness.html"
DIST = ROOT / "target" / "playback-progress-dom"


def build_harness() -> None:
    result = subprocess.run(
        [
            "node",
            str(ROOT / "node_modules" / "vite" / "bin" / "vite.js"),
            "build",
            "--config",
            "vite.playback-harness.config.ts",
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=120,
    )
    if result.returncode != 0:
        raise RuntimeError(f"Vite playback harness build failed:\n{result.stdout}\n{result.stderr}")


def check_playlist_duration(page) -> None:
    page.goto(f"{URL}?scenario=playlist", wait_until="domcontentloaded", timeout=10_000)
    page.wait_for_selector("[data-testid='audio-snapshot-position']", timeout=10_000)
    slider = page.locator("input.progress-slider")
    assert slider.get_attribute("max") == "184000", "playlist metadata duration was lost from the progress range"
    assert not slider.is_disabled(), "known playlist track duration must keep the progress control available"
    assert page.locator(".progress-row span").last.inner_text() == "3:04", "total duration is not rendered"


def check_live_snapshot_after_seek(page) -> None:
    page.goto(f"{URL}?scenario=library", wait_until="domcontentloaded", timeout=10_000)
    position = page.locator("[data-testid='audio-snapshot-position']")
    slider = page.locator("input.progress-slider")
    page.wait_for_function("Number(document.querySelector('[data-testid=audio-snapshot-position]').textContent) > 5000")
    page.wait_for_function("() => document.querySelector('input.progress-slider').value === document.querySelector('[data-testid=audio-snapshot-position]').textContent")
    before = int(position.inner_text())
    assert int(slider.input_value()) == before, "initial UI position must match audio snapshot"
    page.evaluate("""() => {
      const slider = document.querySelector('input.progress-slider');
      slider.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
      slider.value = '23000';
      slider.dispatchEvent(new Event('input', { bubbles: true }));
      slider.dispatchEvent(new Event('change', { bubbles: true }));
      window.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
    }""")
    assert page.locator("[data-testid='seek-request']").inner_text() == "23000", "seek must commit the final input value"
    assert page.locator("[data-testid='seek-request-count']").inner_text() == "1", "change plus pointerup must seek only once"
    page.wait_for_function("Number(document.querySelector('[data-testid=audio-snapshot-position]').textContent) > 6000")
    after = int(position.inner_text())
    rendered = int(slider.input_value())
    assert after > before, "the synthetic audio snapshot must continue advancing"
    assert rendered == after, f"playhead froze at {rendered}ms while the audio snapshot advanced to {after}ms"
    elapsed_text = f"{after // 60_000}:{(after // 1_000) % 60:02d}"
    assert page.locator(".progress-row span").first.inner_text() == elapsed_text, "elapsed display must follow worker time"

    page.evaluate("window.failNextSeek()")
    page.evaluate("""() => {
      const slider = document.querySelector('input.progress-slider');
      slider.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
      slider.value = '30000';
      slider.dispatchEvent(new Event('input', { bubbles: true }));
      slider.dispatchEvent(new Event('change', { bubbles: true }));
      window.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
    }""")
    assert page.locator("[data-testid='seek-error']").inner_text() == "seek unsupported", "seek failure must be visible"
    page.wait_for_function("() => document.querySelector('input.progress-slider').value === document.querySelector('[data-testid=audio-snapshot-position]').textContent")
    after_failure = int(position.inner_text())
    assert int(slider.input_value()) == after_failure, "failed seek must discard its draft and keep the audio snapshot"
    assert page.locator("[data-testid='seek-request-count']").inner_text() == "2"

    current_count = int(page.locator("[data-testid='seek-request-count']").inner_text())
    slider.focus()
    page.keyboard.press("ArrowRight")
    page.wait_for_function("count => Number(document.querySelector('[data-testid=seek-request-count]').textContent) > count", arg=current_count)
    assert int(page.locator("[data-testid='seek-request-count']").inner_text()) == current_count + 1, "keyboard change must send one seek"

    current_count = int(page.locator("[data-testid='seek-request-count']").inner_text())
    page.evaluate("""() => {
      const slider = document.querySelector('input.progress-slider');
      slider.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
      slider.value = '42000';
      slider.dispatchEvent(new Event('input', { bubbles: true }));
      window.setTrack('track-next', 1000);
      window.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }));
    }""")
    page.wait_for_function("document.querySelector('input.progress-slider').value === '1000'")
    assert int(page.locator("[data-testid='seek-request-count']").inner_text()) == current_count, "old drag must not seek into the new track"


def main() -> None:
    build_harness()
    server = ThreadingHTTPServer(
        ("127.0.0.1", 1451),
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
            errors: list[str] = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.on("console", lambda message: errors.append(message.text) if message.type == "error" else None)
            check_playlist_duration(page)
            check_live_snapshot_after_seek(page)
            assert not errors, "browser errors: " + "; ".join(errors)
            browser.close()
        print("Playback progress DOM checks passed: live seek clock and playlist duration.")
    finally:
        server.shutdown()
        server.server_close()
        server_thread.join(timeout=5)


if __name__ == "__main__":
    main()
