import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const appSource = readFileSync(path.join(root, 'src/App.svelte'), 'utf8');
const appStyles = readFileSync(path.join(root, 'src/app.css'), 'utf8');
const trackListSource = readFileSync(path.join(root, 'src/lib/TrackList.svelte'), 'utf8');
const playlistEntrySource = readFileSync(path.join(root, 'src/lib/PlaylistEntryList.svelte'), 'utf8');

function functionSource(startText, endText) {
  const start = appSource.indexOf(startText);
  assert.notEqual(start, -1, `missing ${startText}`);
  const end = appSource.indexOf(endText, start + startText.length);
  assert.notEqual(end, -1, `missing boundary ${endText}`);
  return appSource.slice(start, end);
}

test('playing a library or playlist item does not change the current page', () => {
  const libraryPlay = functionSource('async function playTrack(', 'function openNowPlaying(');
  const playlistPlay = functionSource('async function playPlaylistEntry(', 'async function pickWindowsFolder(');

  assert.match(libraryPlay, /invokeCommand\('playback_play'/);
  assert.match(playlistPlay, /invokeCommand\('playback_play'/);
  assert.doesNotMatch(libraryPlay, /activeView\s*=/);
  assert.doesNotMatch(playlistPlay, /activeView\s*=/);
});

test('the footer artwork is the only Now Playing navigation control', () => {
  const navStart = appSource.indexOf('<nav class="primary-nav"');
  assert.notEqual(navStart, -1, 'missing primary navigation');
  const navEnd = appSource.indexOf('</nav>', navStart);
  assert.notEqual(navEnd, -1, 'primary navigation must close');
  const primaryNav = appSource.slice(navStart, navEnd);
  assert.doesNotMatch(primaryNav, /now-playing|正在播放/);
  assert.equal([...appSource.matchAll(/activeView\s*=\s*'now-playing'/g)].length, 1);

  const artClass = appSource.indexOf('class="dock-art"');
  assert.notEqual(artClass, -1, 'missing footer artwork control');
  const buttonStart = appSource.lastIndexOf('<button', artClass);
  const buttonEnd = appSource.indexOf('</button>', artClass);
  assert.ok(buttonStart >= 0 && buttonEnd > artClass, 'footer artwork must be a button');
  const artworkButton = appSource.slice(buttonStart, buttonEnd + '</button>'.length);
  assert.match(artworkButton, /onclick=\{openNowPlaying\}/);
  assert.match(artworkButton, /disabled=\{!playback\?\.currentTrack\}/);
  assert.match(artworkButton, /aria-label=/);

  const navigationHandler = functionSource('function openNowPlaying(', 'async function togglePlayback(');
  assert.match(navigationHandler, /activeView\s*=\s*'now-playing'/);
});

test('Now Playing hides library sync details and the native playback note', () => {
  const pageContentStart = appSource.indexOf('<div class="page-content">');
  const syncBannerStart = appSource.indexOf('class="sync-progress-banner"', pageContentStart);
  assert.ok(pageContentStart >= 0 && syncBannerStart > pageContentStart, 'library sync banner should remain in the shared page content');
  const syncCondition = appSource.slice(pageContentStart, syncBannerStart);
  assert.match(syncCondition, /\{#if syncProgress && activeView !== 'now-playing'\}/);

  const nowPlayingStart = appSource.indexOf("{:else if activeView === 'now-playing'}");
  const settingsStart = appSource.indexOf('<section class="settings-page"', nowPlayingStart);
  assert.ok(nowPlayingStart >= 0 && settingsStart > nowPlayingStart, 'Now Playing markup should be bounded before the settings page');
  const nowPlayingMarkup = appSource.slice(nowPlayingStart, settingsStart);
  assert.doesNotMatch(nowPlayingMarkup, /playback-note|播放狀態由原生音訊服務提供/);
});

test('Tabler icons keep the Chinese playback labels and settings navigation accessible', () => {
  assert.match(appSource, /from '@tabler\/icons-svelte-runes'/);

  const settingsStart = appSource.indexOf("class:active={activeView === 'settings'}");
  const settingsEnd = appSource.indexOf('<span class="nav-label">設定</span>', settingsStart);
  assert.ok(settingsStart >= 0 && settingsEnd > settingsStart, 'settings navigation entry should remain available');
  assert.match(appSource.slice(settingsStart, settingsEnd), /<IconSettings[^>]+aria-hidden="true"/);

  const controlsStart = appSource.indexOf('<div class="dock-controls">');
  const controlsEnd = appSource.indexOf('</div>\n      <PlaybackProgress', controlsStart);
  assert.ok(controlsStart >= 0 && controlsEnd > controlsStart, 'footer playback controls should remain available');
  const controls = appSource.slice(controlsStart, controlsEnd);
  assert.match(controls, /<IconArrowsShuffle[^>]+aria-hidden="true"/);
  assert.match(controls, /<IconPlayerTrackPrev[^>]+aria-hidden="true"/);
  assert.match(controls, /<IconPlayerTrackNext[^>]+aria-hidden="true"/);
  assert.match(controls, /<IconPlayerPause[^>]+aria-hidden="true"/);
  assert.match(controls, /<IconPlayerPlay[^>]+aria-hidden="true"/);
  assert.match(controls, /<IconRepeatOff[^>]+aria-hidden="true"/);
  assert.match(controls, /<IconRepeatOnce[^>]+aria-hidden="true"/);
  assert.match(controls, /<IconRepeat[^>]+aria-hidden="true"/);
  assert.match(controls, /aria-label="隨機播放"/);
  assert.match(controls, /aria-label="上一首"/);
  assert.match(controls, /aria-label=\{playback\?\.isPlaying \? '暫停' : '播放'\}/);
  assert.match(controls, /aria-label="下一首"/);
  assert.match(controls, /aria-label="循環播放"/);
  assert.doesNotMatch(controls, /<svg/);
});

test('visible UI icons use Tabler components except the original brand mark', () => {
  const inlineSvgPositions = [...appSource.matchAll(/<svg\b/g)].map((match) => match.index);
  assert.equal(inlineSvgPositions.length, 1, 'only the custom brand logo may remain inline SVG');
  assert.match(appSource.slice(0, inlineSvgPositions[0]), /class="brand-mark"[\s\S]{0,180}$/);
  assert.doesNotMatch(trackListSource, /<svg\b/);
  assert.doesNotMatch(playlistEntrySource, /<svg\b/);

  const iconGlyphs = /[\u203a\u2315\u266a\u266b\u2191\u2193]/;
  assert.doesNotMatch(appSource, iconGlyphs, 'App markup should not use hand-drawn icon characters');
  assert.doesNotMatch(trackListSource, iconGlyphs, 'track rows should not use icon characters');
  assert.doesNotMatch(playlistEntrySource, iconGlyphs, 'playlist rows should not use icon characters');
  assert.doesNotMatch(appStyles, /content\s*:\s*['"]\u2713['"]/);

  for (const name of [
    'IconAlertCircle', 'IconArrowDown', 'IconArrowUp', 'IconCheck', 'IconChevronRight',
    'IconFilter', 'IconFolder', 'IconHeart', 'IconLibrary', 'IconMusic', 'IconPlaylist',
    'IconRefresh', 'IconSearch', 'IconVolume2',
  ]) {
    assert.match(appSource, new RegExp(`<${name}\\b`), `${name} should be used by the interface`);
  }
  assert.match(trackListSource, /<IconPlayerPlayFilled\b/);
  assert.match(playlistEntrySource, /<IconPlayerPlayFilled\b/);

  assert.match(appSource, /aria-label="搜尋曲庫"/);
  assert.match(appSource, /title="上移欄位"/);
  assert.match(appSource, /title="下移欄位"/);
  assert.match(appSource, /aria-label="音量"/);
  assert.match(trackListSource, /aria-label=\{`播放 \$\{row\.item\.title/);
  assert.match(playlistEntrySource, /aria-label=\{`播放 \$\{row\.item\.title/);
});
