import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const readSource = (relativePath) => readFileSync(path.join(root, relativePath), 'utf8').replace(/\r\n/g, '\n');
const appSource = readSource('src/App.svelte');
const appStyles = readSource('src/app.css');
const trackColumnCellsSource = readSource('src/lib/TrackColumnCells.svelte');
const trackListSource = readSource('src/lib/TrackList.svelte');
const playlistEntrySource = readSource('src/lib/PlaylistEntryList.svelte');
const playlistTreeSource = readSource('src/lib/PlaylistTree.svelte');
const favoritePopoverSource = readSource('src/lib/FavoritePopover.svelte');

function functionSource(startText, endText) {
  const start = appSource.indexOf(startText);
  assert.notEqual(start, -1, `missing ${startText}`);
  const end = appSource.indexOf(endText, start + startText.length);
  assert.notEqual(end, -1, `missing boundary ${endText}`);
  return appSource.slice(start, end);
}

test('previous button seeks to the start after three seconds and navigates before that', () => {
  const handler = functionSource('async function controlPlayback(', 'async function commitPlaybackSeek(');
  assert.match(handler, /command === 'playback_previous' && \(playback\?\.positionMs \?\? 0\) > 3000/);
  assert.match(handler, /invokeCommand\('playback_seek', \{ positionMs: 0 \}\)/);
  assert.match(handler, /return;[\s\S]*invokeCommand\(command, \{\}\)/);
});

test('resizable list header separators remain visible without hover', () => {
  const baseRule = /\.column-resize-handle::after\s*\{[^}]*background:\s*rgba\(var\(--text-rgb\),\s*0\.3\)/;
  const hoverRule = /\.list-column\.is-resizable:hover \.column-resize-handle::after,[\s\S]*?\.column-resize-handle:hover::after\s*\{[^}]*background:\s*rgba\(var\(--text-rgb\),\s*0\.65\)/;
  assert.match(trackColumnCellsSource, baseRule);
  assert.match(trackColumnCellsSource, hoverRule);
});

test('playing a library or playlist item does not change the current page', () => {
  const libraryPlay = functionSource('async function playTrack(', 'async function loadArtistIndexRowsForNavigation(');
  const playlistPlay = functionSource('async function playPlaylistEntry(', 'async function pickWindowsFolder(');

  assert.match(libraryPlay, /invokeCommand\('playback_play'/);
  assert.match(playlistPlay, /invokeCommand\('playback_play'/);
  assert.doesNotMatch(libraryPlay, /activeView\s*=/);
  assert.doesNotMatch(playlistPlay, /activeView\s*=/);
});

test('the footer artwork opens and closes a full-window Now Playing overlay', () => {
  const navStart = appSource.indexOf('<nav class="primary-nav"');
  assert.notEqual(navStart, -1, 'missing primary navigation');
  const navEnd = appSource.indexOf('</nav>', navStart);
  assert.notEqual(navEnd, -1, 'primary navigation must close');
  const primaryNav = appSource.slice(navStart, navEnd);
  assert.doesNotMatch(primaryNav, /now-playing|正在播放/);
  assert.doesNotMatch(appSource, /activeView\s*=\s*'now-playing'/);

  const artClass = appSource.indexOf('class="dock-art"');
  assert.notEqual(artClass, -1, 'missing footer artwork control');
  const buttonStart = appSource.lastIndexOf('<button', artClass);
  const buttonEnd = appSource.indexOf('</button>', artClass);
  assert.ok(buttonStart >= 0 && buttonEnd > artClass, 'footer artwork must be a button');
  const artworkButton = appSource.slice(buttonStart, buttonEnd + '</button>'.length);
  assert.match(artworkButton, /`開啟正在播放/);
  assert.doesNotMatch(artworkButton, /isNowPlayingOpen\s*\?\s*'返回播放前頁面'/);
  assert.match(artworkButton, /onclick=\{\(\)\s*=>\s*void\s*openNowPlaying\(\)\}/);
  assert.match(appSource, /class="dock-now-playing-dismiss"/);
  assert.match(appSource, /class:dock-track-dismiss=\{isNowPlayingOpen\}/);
  assert.match(artworkButton, /disabled=\{!playback\?\.currentTrack\}/);
  assert.match(artworkButton, /aria-label=/);

  const openHandler = functionSource('async function openNowPlaying(', 'async function closeNowPlaying(');
  const closeHandler = functionSource('async function closeNowPlaying(', 'async function togglePlayback(');
  assert.match(openHandler, /nowPlayingReturnView\s*=\s*activeView/);
  assert.match(openHandler, /isNowPlayingMounted\s*=\s*true/);
  assert.match(openHandler, /isNowPlayingOpen\s*=\s*true/);
  assert.doesNotMatch(openHandler, /activeView\s*=/, 'opening Now Playing should preserve the originating route');
  assert.match(closeHandler, /activeView\s*=\s*nowPlayingReturnView/);
  assert.match(closeHandler, /isNowPlayingOpen\s*=\s*false/);

  assert.match(appSource, /class="sidebar"[^>]*inert=\{isNowPlayingOpen\}/);
  assert.match(appSource, /class="workspace"[^>]*inert=\{isNowPlayingOpen\}/);
  assert.match(appSource, /class="now-playing-overlay"[\s\S]*?aria-hidden=\{!isNowPlayingOpen\}[\s\S]*?inert=\{!isNowPlayingOpen\}/);
  assert.match(appSource, /class="[^"]*\bnow-playing-overlay-return\b[^"]*"[\s\S]*?aria-label="返回播放前頁面"[\s\S]*?onclick=\{\(\)\s*=>\s*void closeNowPlaying\(\)\}/);
});

test('the sidebar playlist group is the single playlist navigation entry and works when empty', () => {
  const navStart = appSource.indexOf('<nav class="primary-nav"');
  const navEnd = appSource.indexOf('</nav>', navStart);
  assert.ok(navStart >= 0 && navEnd > navStart, 'primary navigation must remain available');
  assert.doesNotMatch(appSource.slice(navStart, navEnd), /播放清單|設定/);

  const treeHost = /<div class="sidebar-playlist-tree-host">\s*<PlaylistTree([\s\S]*?)\/>\s*<\/div>/.exec(appSource);
  assert.ok(treeHost, 'playlist tree must always render in the sidebar');
  assert.match(treeHost[1], /onOpen=/, 'playlist group heading must open the playlist page');
  assert.match(playlistTreeSource, /aria-label=\{expanded \? '收合播放清單' : '展開播放清單'\}/);
  assert.match(appSource, /我的播放清單．\$\{selectedPlaylist\.name/, 'playlist top bar must include the selected playlist name');
  assert.doesNotMatch(appSource, /playlist-import-button|匯入 M3U\/M3U8|exportPlaylist|importPlaylist/, 'playlist page must not show M3U import/export UI');
  assert.doesNotMatch(appSource, /\{#if playlists\.length > 0\}\s*<div class="sidebar-playlist-tree-host"/);
});

test('reopening the selected playlist does not reset its retained virtual list', () => {
  const selectPlaylist = functionSource('async function selectPlaylist(', 'function refreshPlaylistEntries(');
  assert.match(selectPlaylist, /const playlistChanged = selectedPlaylistId !== playlistId/);
  assert.match(selectPlaylist, /openPlaylistPage\(\)/);
  assert.match(selectPlaylist, /if \(playlistChanged\)[\s\S]*?playlistListRevision\s*\+=\s*1/);
  assert.doesNotMatch(selectPlaylist, /playlistListRevision\s*\+=\s*1[\s\S]*activeView\s*=/);
});

test('playlist page keeps one mounted list instance and refreshes only when its selected data changes', () => {
  const playlistSection = appSource.slice(
    appSource.indexOf('{#if visitedListViews.playlists}'),
    appSource.indexOf('{#if visitedListViews.queue}'),
  );
  assert.match(playlistSection, /hidden=\{activeView !== 'playlists'\}/);
  assert.doesNotMatch(playlistSection, /\{#if activeView\s*===\s*'playlists'\}/);
  assert.doesNotMatch(playlistSection, /\{#key selectedPlaylist\.id\}/);
  assert.match(playlistEntrySource, /scrollResetKey=\{playlistId\}/);

  const loadPlaylists = functionSource('async function loadPlaylists(', 'async function selectPlaylist(');
  assert.match(loadPlaylists, /previousPlaylistId !== nextId \|\| previousEntryCount !== nextEntryCount/);
  assert.match(loadPlaylists, /playlistListRevision\s*\+=\s*1/);
  assert.doesNotMatch(loadPlaylists, /selectedPlaylistId = nextId;\s*playlistListRevision\s*\+=\s*1/);
  const openPlaylistPage = functionSource('function openPlaylistPage(', 'function ');
  assert.match(openPlaylistPage, /visitedListViews\.playlists = true/);
  assert.match(openPlaylistPage, /activeView = 'playlists'/);
});

test('Now Playing overlays the retained route without duplicating sync or native playback details', () => {
  const sourcesPanel = appSource.indexOf('id="{scope}sources-panel"');
  const syncBannerStart = appSource.indexOf('class="sync-progress-banner"', sourcesPanel);
  assert.ok(sourcesPanel >= 0 && syncBannerStart > sourcesPanel, 'library sync banner belongs on the music-sources settings panel');
  const syncCondition = appSource.slice(sourcesPanel, syncBannerStart);
  assert.match(syncCondition, /\{#if syncProgress\}/);
  const librarySection = appSource.indexOf('class="library-section"');
  const playlistSection = appSource.indexOf('class="playlist-section"', librarySection);
  assert.doesNotMatch(appSource.slice(librarySection, playlistSection), /sync-progress-banner/);
  assert.doesNotMatch(appSource, /LIBRARY SYNC|你的音樂空間|LOCAL FIRST|MoeMusicPlayer <span>0\.1/);

  const overlayStart = appSource.indexOf('class="now-playing-overlay"');
  const nowPlayingStart = appSource.indexOf('<section class="now-playing-view"', overlayStart);
  const overlayEnd = appSource.indexOf('</section>\n  {/if}', nowPlayingStart);
  assert.ok(overlayStart >= 0 && nowPlayingStart > overlayStart && overlayEnd > nowPlayingStart, 'Now Playing markup should live in its own overlay');
  const nowPlayingMarkup = appSource.slice(nowPlayingStart, overlayEnd);
  assert.doesNotMatch(nowPlayingMarkup, /sync-progress-banner/);
  assert.doesNotMatch(nowPlayingMarkup, /playback-note|播放狀態由原生音訊服務提供/);
  assert.doesNotMatch(appSource, /返回曲庫/, 'Now Playing should not hardcode library as the return destination');
});

test('Tabler icons keep the Chinese playback labels and settings navigation accessible', () => {
  assert.match(appSource, /from '@tabler\/icons-svelte-runes'/);

  const settingsStart = appSource.indexOf('class="sidebar-settings"');
  assert.ok(settingsStart > appSource.indexOf('class="sidebar-playlist-tree-host"'), 'settings stays at the bottom of the sidebar');
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
  assert.equal([...appSource.matchAll(/<svg\b/g)].length, 0, 'App markup should use the SilverWolf image instead of an inline brand SVG');
  assert.match(appSource, /SilverWolfIcon\.png/);
  assert.match(appSource, /<strong>MOE<\/strong>\s*<span>Music Otaku Edge<\/span>/);
  assert.doesNotMatch(trackListSource, /<svg\b/);
  assert.doesNotMatch(playlistEntrySource, /<svg\b/);

  const iconGlyphs = /[\u203a\u2315\u266a\u266b\u2191\u2193]/;
  assert.doesNotMatch(appSource, iconGlyphs, 'App markup should not use hand-drawn icon characters');
  assert.doesNotMatch(trackListSource, iconGlyphs, 'track rows should not use icon characters');
  assert.doesNotMatch(playlistEntrySource, iconGlyphs, 'playlist rows should not use icon characters');
  assert.doesNotMatch(appStyles, /content\s*:\s*['"]\u2713['"]/);

  for (const name of [
    'IconAlertCircle', 'IconArrowDown', 'IconArrowUp', 'IconChevronRight',
    'IconFolder', 'IconHeart', 'IconLibrary', 'IconMusic',
    'IconRefresh', 'IconSearch', 'IconVolume2',
  ]) {
    const source = name === 'IconHeart' ? favoritePopoverSource : appSource;
    assert.match(source, new RegExp(`<${name}\\b`), `${name} should be used by the interface`);
  }
  assert.match(playlistTreeSource, /<IconPlaylist\b/);
  assert.doesNotMatch(trackListSource, /<IconPlayerPlayFilled\b/);
  assert.doesNotMatch(playlistEntrySource, /<IconPlayerPlayFilled\b/);

  assert.match(appSource, /class="sidebar-search-field"/);
  assert.match(appSource, /aria-label="搜尋曲庫"/);
  assert.doesNotMatch(appSource, /篩選功能尚未接通/);
  assert.match(appSource, /title="上移欄位"/);
  assert.match(appSource, /title="下移欄位"/);
  assert.match(appSource, /aria-label="音量"/);
  assert.match(trackListSource, /ariaLabel="曲庫曲目；使用方向鍵瀏覽，按 Enter 播放目前曲目；點擊欄位標題排序"/);
  assert.match(playlistEntrySource, /ariaLabel="播放清單項目；點擊欄位標題排序；使用方向鍵瀏覽，按 Enter 播放目前項目"/);
  assert.match(playlistEntrySource, /sortableIndex=\{true\}/);
  assert.match(playlistEntrySource, /sort: PlaylistListSort/);
});
