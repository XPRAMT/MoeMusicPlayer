import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('About page places the update check button below the current status', async () => {
  const aboutPage = await readFile(new URL('../src/lib/AboutPage.svelte', import.meta.url), 'utf8');
  const sectionStart = aboutPage.indexOf('<section class="about-updates"');
  const sectionEnd = aboutPage.indexOf('<section class="about-credits"', sectionStart);
  assert.notEqual(sectionStart, -1);
  assert.notEqual(sectionEnd, -1);
  const section = aboutPage.slice(sectionStart, sectionEnd);
  const statusPosition = section.indexOf('<p role="status" aria-live="polite">{updateStatusText(updateState)}</p>');
  const buttonPosition = section.indexOf('onclick={checkUpdate}');
  assert.notEqual(statusPosition, -1);
  assert.notEqual(buttonPosition, -1);
  assert.ok(statusPosition < buttonPosition);
  assert.ok(buttonPosition > section.indexOf('</div>'));
});
