import './helpers/untr-fs.mjs';
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
const pagedVirtualListSource = readSource('src/lib/PagedVirtualList.svelte');
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
  assert.match(handler, /command === 'playback_previous' && currentPlaybackPositionMs\(\) > 3000/);
  assert.match(handler, /seekPlayback\(0\)/);
  assert.match(handler, /return;[\s\S]*invokeCommand\(command, \{\}\)/);
});

test('queue heading sort mutates playback traversal and keeps drag reordering enabled', () => {
  const queueList = readSource('src/lib/PlaybackQueueList.svelte');
  assert.match(appSource, /invokeCommand\('playback_sort_queue', \{ sort: nextSort \}\)/);
  assert.match(queueList, /longPressReorder=\{true\}/);
  assert.match(queueList, /onReorder=\{handleReorder\}/);
  assert.match(appSource, /hideSortDirection=\{playback\?\.shuffle === true\}/);
  assert.doesNotMatch(appSource, /queueSort = null/);
  assert.match(appSource, /if \(Object\.hasOwn\(resolved, 'queueSort'\)\) queueSort = resolved\.queueSort \?\? null/);
  assert.match(readSource('src/lib/TrackColumnCells.svelte'), /sort\?\.field === column\.id && !hideSortDirection/);
});

test('column resize handle follows divider alpha without a second opacity multiplier', () => {
  const handleRule = trackColumnCellsSource.match(/\.column-resize-handle::after\s*\{([^}]+)\}/)?.[1] ?? '';
  const hoverRule = trackColumnCellsSource.match(/\.list-column\.is-resizable:hover \.column-resize-handle::after,[\s\S]*?\.column-resize-handle:hover::after\s*\{([^}]+)\}/)?.[1] ?? '';
  assert.match(handleRule, /width:\s*1px/);
  assert.match(handleRule, /background:\s*var\(--divider-color\)/);
  assert.doesNotMatch(handleRule, /opacity\s*:/);
  assert.equal(hoverRule, '');
});

test('playing a library or playlist item does not change the current page', () => {
  const libraryPlay = functionSource('async function playTrack(', 'async function resolveArtistEntryByKey(');
  const playlistPlay = functionSource('async function playPlaylistEntry(', 'async function playQueueEntry(');

  assert.match(libraryPlay, /invokeCommand\('playback_play'/);
  assert.match(playlistPlay, /invokeCommand\('playback_play'/);
  assert.doesNotMatch(libraryPlay, /activeView\s*=/);
  assert.doesNotMatch(playlistPlay, /activeView\s*=/);
});

test('artist and album indexes use SQLite snapshots without renderer-side identity facets', () => {
  const artistIndex = readSource('src/lib/ArtistIndex.svelte');
  const albumIndex = readSource('src/lib/AlbumIndex.svelte');
  assert.match(artistIndex, /library_create_artist_index_snapshot/);
  assert.match(albumIndex, /library_create_album_index_snapshot/);
  assert.match(artistIndex, /library_get_artist_index_snapshot_page/);
  assert.match(albumIndex, /library_get_album_index_snapshot_page/);
  assert.doesNotMatch(appSource + artistIndex + albumIndex, /loadIndexIdentityMappings|library_get_artist_index_facets|library_get_album_index_facets|artist-identities\.js|album-identities\.js/);
});

test('detail navigation resolves one backend index row instead of loading the full index into App', () => {
  assert.match(appSource, /library_resolve_artist_index_entry/);
  assert.match(appSource, /library_resolve_album_index_entry/);
  assert.doesNotMatch(appSource, /loadArtistIndexRowsForNavigation|loadAlbumIndexRowsForNavigation|library_get_artist_index\s*,|buildAlbumEntries\(/);
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
  assert.doesNotMatch(artworkButton, /onclick=/, 'artwork should use the shared dock click handler');
  assert.match(appSource, /<footer class="player-dock"[^>]*onclick=\{toggleNowPlayingFromDock\}/);
  assert.match(appSource, /target\.closest\('button:not\(\.dock-art\), a, input, textarea, select, \.progress-slider, \.dock-controls, \.volume-popover'\)/);
  assert.match(appSource, /class="dock-now-playing-dismiss"/);
  assert.match(appSource, /class:dock-track-dismiss=\{isNowPlayingOpen\}/);
  assert.match(artworkButton, /disabled=\{!playback\?\.currentTrack\}/);
  assert.match(artworkButton, /aria-label=/);
  assert.doesNotMatch(artworkButton, /use:appTooltip/);
  assert.doesNotMatch(artworkButton, /查看正在播放/);

  const openHandler = functionSource('async function openNowPlaying(', 'async function closeNowPlaying(');
  const closeHandler = functionSource('async function closeNowPlaying(', 'async function togglePlayback(');
  assert.match(openHandler, /nowPlayingReturnView\s*=\s*activeView/);
  assert.match(openHandler, /isNowPlayingMounted\s*=\s*true/);
  assert.match(openHandler, /isNowPlayingOpen\s*=\s*true/);
  assert.doesNotMatch(openHandler, /activeView\s*=/, 'opening Now Playing should preserve the originating route');
  assert.match(closeHandler, /activeView\s*=\s*nowPlayingReturnView/);
  assert.match(closeHandler, /isNowPlayingOpen\s*=\s*false/);

  assert.match(appSource, /class="sidebar"[\s\S]*?inert=\{isNowPlayingOpen \|\| \(isNarrowNav && !isNavPanelOpen\)\}/);
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
  // Playlists are always expanded: no collapse toggle or state.
  assert.doesNotMatch(playlistTreeSource, /playlist-tree-toggle|expanded|matchMedia/);
  assert.match(playlistTreeSource, /<\/div>\s*<ul id="sidebar-playlist-items"/);
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

test('sidebar list routes remount for fresh data and restore each virtual list position', () => {
  assert.match(appSource, /\{#if activeView === 'library'\}[\s\S]*?<section class="library-section"/);
  assert.match(appSource, /\{#if activeView === 'artists' \|\| \(activeView === 'artist-detail' && keepArtistIndexForDetail\)\}/);
  assert.match(appSource, /\{#if activeView === 'artist-detail' && selectedArtist\}/);
  assert.match(appSource, /\{#if activeView === 'playlists'\}[\s\S]*?<section class="playlist-section"/);
  assert.match(appSource, /\{#if activeView === 'queue'\}[\s\S]*?<section class="queue-page"/);
  assert.doesNotMatch(appSource, /visitedListViews/);
  assert.match(appSource, /keepArtistIndexForDetail = preserveIndex/);
  assert.match(appSource, /onSelect=\{\(artist\) => openArtistEntry\(artist, true\)\}/);
  assert.match(appSource, /listId=\{`artist-detail-\$\{selectedArtist\.key\}`\}/);

  assert.match(playlistEntrySource, /scrollResetKey=\{playlistId\}/);
  assert.match(playlistEntrySource, /scrollRestoreReady=\{snapshot\.scope === JSON\.stringify\(\{ playlistId, sort \}\)/);
  assert.match(pagedVirtualListSource, /SCROLL_POSITION_CACHE_LIMIT = 64/);
  assert.match(pagedVirtualListSource, /const listChanged = seenListId !== undefined && currentListId !== seenListId/);
  assert.match(pagedVirtualListSource, /if \(seenListId === undefined \|\| listChanged\)/);
  assert.match(pagedVirtualListSource, /pendingListRestore = \{ listId: currentListId, position \}/);
  assert.match(pagedVirtualListSource, /getRestoredListScrollTop\(\{[\s\S]*?groupingReady,[\s\S]*?virtualTotalCount,[\s\S]*?\}\)/);
  assert.match(pagedVirtualListSource, /if \(restoredScrollTop !== null && scrollRestoreReady\)/);
  assert.match(pagedVirtualListSource, /hasPreviousGroupSnapshot = false/);
  assert.match(pagedVirtualListSource, /shouldAdjustGroupAnchor\(\{[\s\S]*?hasPreviousSnapshot: hasPreviousGroupSnapshot/);
  assert.match(pagedVirtualListSource, /hasPreviousGroupSnapshot = true/);
  assert.match(pagedVirtualListSource, /const safeRestoredScrollTop = Math\.min\(restoredScrollTop, maximum\)/);
  assert.match(pagedVirtualListSource, /writeListScrollPosition\(currentListId, safeRestoredScrollTop\)/);
  assert.match(pagedVirtualListSource, /let lastVisibleScrollTop = 0/);
  assert.match(pagedVirtualListSource, /if \(viewport && viewport\.clientHeight > 0\)\s*\{\s*viewport\.scrollTop = nativeScrollTopFor\(safeRestoredScrollTop, restoredCollapseOffset\)/);
  assert.match(pagedVirtualListSource, /if \(viewport && nextViewportHeight > 0\)[\s\S]*?lastVisibleScrollTop = scrollTop/);

  const loadPlaylists = functionSource('async function loadPlaylists(', 'async function selectPlaylist(');
  assert.match(loadPlaylists, /previousPlaylistId !== nextId \|\| previousEntryCount !== nextEntryCount/);
  assert.match(loadPlaylists, /playlistListRevision\s*\+=\s*1/);
  assert.doesNotMatch(loadPlaylists, /selectedPlaylistId = nextId;\s*playlistListRevision\s*\+=\s*1/);
  const openPlaylistPage = functionSource('function openPlaylistPage(', 'async function playPlaylistEntry(');
  assert.match(openPlaylistPage, /activeView = 'playlists'/);
});

test('visible virtual rows depend on the page revision even when the range is unchanged', () => {
  const virtualRows = /const virtualRows = \$derived\(buildPagedVirtualRows\(\{([\s\S]*?)\}\)\);/.exec(pagedVirtualListSource);
  assert.ok(virtualRows, 'visible rows should be derived through the shared paged-row builder');
  assert.match(virtualRows[1], /\brevision\s*,/, 'page cache changes must invalidate visible rows');
  assert.match(pagedVirtualListSource, /#each virtualRows\.rows as virtualRow/);
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
    'IconAlertCircle', 'IconChevronRight', 'IconDisc',
    'IconFolder', 'IconHeart', 'IconMusic',
    'IconRefresh', 'IconSearch', 'IconVolume2',
  ]) {
    const source = name === 'IconHeart' ? favoritePopoverSource : appSource;
    assert.match(source, new RegExp(`<${name}\\b`), `${name} should be used by the interface`);
  }
  assert.match(playlistTreeSource, /<IconPlaylist\b/);
  assert.doesNotMatch(trackListSource, /<IconPlayerPlayFilled\b/);
  assert.doesNotMatch(playlistEntrySource, /<IconPlayerPlayFilled\b/);

  assert.match(appSource, /class="sidebar-search-field"/);
  assert.match(appSource, /<IconMusic size=\{20\} stroke=\{1\.6\} aria-hidden="true" \/>\s*<span class="nav-label">曲庫/);
  assert.match(appSource, /<IconDisc size=\{20\} stroke=\{1\.6\} aria-hidden="true" \/>\s*<span class="nav-label">專輯/);
  assert.match(appSource, /aria-label="搜尋曲庫"/);
  assert.doesNotMatch(appSource, /篩選功能尚未接通/);
  assert.match(appSource, /use:appTooltip=\{'長按 300 毫秒後拖曳排序/);
  assert.match(appSource, /use:appTooltip=\{'長按 300 毫秒後拖曳排序/);
  assert.match(appSource, /aria-label="音量"/);
  assert.match(trackListSource, /ariaLabel=\{sortable\s+\?\s+'曲庫曲目；使用方向鍵瀏覽，按 Enter 播放目前曲目；點擊欄位標題排序'/);
  assert.match(trackListSource, /'曲目；使用方向鍵瀏覽，按 Enter 播放目前曲目'/);
  assert.match(playlistEntrySource, /ariaLabel="播放清單項目；點擊欄位標題排序；使用方向鍵瀏覽，按 Enter 播放目前項目"/);
  assert.match(playlistEntrySource, /sortableIndex=\{true\}/);
  assert.match(playlistEntrySource, /sort: PlaylistListSort/);
});

test('sidebar nav items scroll while 設定 stays pinned, and narrow widths use the MOE logo panel', async () => {
  const css = await (await import('node:fs/promises')).readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  const scrollStart = appSource.indexOf('<div class="sidebar-scroll">');
  const settingsStart = appSource.indexOf('<nav class="sidebar-settings"');
  assert.ok(scrollStart >= 0 && settingsStart > scrollStart);
  const scrollBlock = appSource.slice(scrollStart, settingsStart);
  assert.match(scrollBlock, /<nav class="primary-nav">[\s\S]*<PlaylistTree/);
  assert.match(css, /\.sidebar-scroll \{[^}]*overflow-y: auto;/s);
  assert.match(appSource, /class="nav-panel-toggle"[\s\S]*?aria-expanded=\{isNavPanelOpen\}[\s\S]*?<img src=\{brandIcon\}/);
  assert.match(appSource, /class="nav-panel-scrim"[^>]*onclick=\{\(\) => closeNavPanel\('outside', false\)\}/);
  assert.doesNotMatch(css, /grid-template-rows: 58px/, 'narrow widths must not move navigation into a top bar');
  assert.doesNotMatch(css, /--sidebar-width: 72px/, 'the icon rail layout is gone');
});

test('top-bar refresh collapses to an icon at the narrow breakpoint but keeps its accessible name', async () => {
  const css = await (await import('node:fs/promises')).readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  const narrow = css.slice(css.indexOf('@media (max-width: 820px) {'));
  assert.match(narrow.slice(0, narrow.indexOf('\n}')), /\.soft-button\.topbar-refresh-button > span \{ display: none; \}/);
  assert.match(appSource, /class="soft-button topbar-refresh-button"[\s\S]*?aria-label="重新整理音樂來源"[\s\S]*?use:appTooltip=/);
});

test('narrow MOE toggle uses one shared gap for window edge → logo and logo → title', async () => {
  const css = await (await import('node:fs/promises')).readFile(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(css, /--nav-toggle-gap: 14px;/);
  assert.match(css, /--nav-toggle-inset-left: calc\(var\(--nav-toggle-icon-size\) \* 73 \/ 1254\);/);
  assert.match(css, /margin-left: calc\(var\(--nav-toggle-gap\) - var\(--nav-toggle-inset-left\)\);/);
  assert.match(css, /margin-right: calc\(var\(--nav-toggle-gap\) - var\(--nav-toggle-inset-right\) - var\(--topbar-gap\)\);/);
  assert.match(css, /\.app-shell \.workspace > \.topbar \{ padding-left: 0; \}/);
});
