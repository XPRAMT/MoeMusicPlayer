<script lang="ts">
  import type { LyricsPreferences, NowPlayingAppearancePreferences, NowPlayingLayout } from './ipc';
  import NowPlayingLayoutSwitch from './NowPlayingLayoutSwitch.svelte';

  interface Props {
    layout: NowPlayingLayout;
    appearance: NowPlayingAppearancePreferences;
    lyrics: LyricsPreferences;
    onLayoutChange: (layout: NowPlayingLayout) => void;
    onAppearanceChange: (patch: Partial<NowPlayingAppearancePreferences>, flush?: boolean) => void;
    onLyricsChange: (patch: Partial<LyricsPreferences>, flush?: boolean) => void;
    onLyricsReset: () => void;
    groups?: 'playback' | 'lyrics' | 'both';
  }

  let {
    layout, appearance, lyrics,
    onLayoutChange, onAppearanceChange, onLyricsChange, onLyricsReset, groups = 'both',
  }: Props = $props();
</script>

<div class="quick-settings-controls">
  {#if groups === 'playback' || groups === 'both'}
  <section class="quick-settings-group" aria-labelledby="quick-settings-playback-heading">
    <h3 id="quick-settings-playback-heading">播放頁</h3>
    <NowPlayingLayoutSwitch {layout} onChange={onLayoutChange} />
    <div class="cover-corner-setting" role="group" aria-labelledby="cover-corner-style-label">
      <span id="cover-corner-style-label" class="cover-corner-setting-label"><strong>專輯封面造型</strong></span>
      <div class="layout-choice-row cover-corner-choice-row">
        <button
          type="button"
          aria-label="專輯封面圓角"
          aria-pressed={appearance.coverCornerStyle === 'rounded'}
          class:chosen={appearance.coverCornerStyle === 'rounded'}
          onclick={() => onAppearanceChange({ coverCornerStyle: 'rounded' }, true)}
        >
          <strong>圓角</strong><span>專輯封面使用圓角</span>
        </button>
        <button
          type="button"
          aria-label="專輯封面方形"
          aria-pressed={appearance.coverCornerStyle === 'square'}
          class:chosen={appearance.coverCornerStyle === 'square'}
          onclick={() => onAppearanceChange({ coverCornerStyle: 'square' }, true)}
        >
          <strong>方形</strong><span>專輯封面不圓角</span>
        </button>
      </div>
    </div>
    <label class="lyrics-preference-range">
      <span><strong>封面背景模糊</strong><output>{appearance.backgroundBlurPx}px</output></span>
      <input type="range" min="0" max="50" step="1" value={appearance.backgroundBlurPx} aria-label="封面背景模糊程度" oninput={(event) => onAppearanceChange({ backgroundBlurPx: Number(event.currentTarget.value) })} onchange={(event) => onAppearanceChange({ backgroundBlurPx: Number(event.currentTarget.value) }, true)} />
    </label>
    <label class="lyrics-preference-range">
      <span><strong>封面背景透明度</strong><output>{appearance.backgroundOpacityPercent}%</output></span>
      <input type="range" min="0" max="100" step="1" value={appearance.backgroundOpacityPercent} aria-label="封面背景透明度" oninput={(event) => onAppearanceChange({ backgroundOpacityPercent: Number(event.currentTarget.value) })} onchange={(event) => onAppearanceChange({ backgroundOpacityPercent: Number(event.currentTarget.value) }, true)} />
    </label>
  </section>
  {/if}

  {#if groups === 'lyrics' || groups === 'both'}
  <section class="quick-settings-group" aria-labelledby="quick-settings-lyrics-heading">
    <div class="quick-settings-group-heading">
      <h3 id="quick-settings-lyrics-heading">歌詞外觀</h3>
      <button class="outline-button" type="button" onclick={onLyricsReset}>恢復預設</button>
    </div>
    <label class="lyrics-preference-toggle">
      <input type="checkbox" checked={lyrics.showTranslation} onchange={(event) => onLyricsChange({ showTranslation: event.currentTarget.checked }, true)} />
      <span><strong>顯示譯文</strong><small>在每行原文下方顯示翻譯</small></span>
    </label>
    <label class="lyrics-preference-toggle">
      <input type="checkbox" checked={lyrics.showRomanization} onchange={(event) => onLyricsChange({ showRomanization: event.currentTarget.checked }, true)} />
      <span><strong>顯示羅馬拼音</strong><small>在每行原文下方顯示拼音</small></span>
    </label>
    <label class="lyrics-preference-toggle">
      <input type="checkbox" checked={lyrics.simplifiedToTraditional} onchange={(event) => onLyricsChange({ simplifiedToTraditional: event.currentTarget.checked }, true)} />
      <span><strong>簡體轉繁體</strong><small>播放時把整首歌詞轉成繁體字形，不改用詞，也不另存檔案</small></span>
    </label>
    <div class="cover-corner-setting" role="group" aria-labelledby="lyrics-text-effect-label">
      <span id="lyrics-text-effect-label" class="cover-corner-setting-label"><strong>歌詞文字效果</strong></span>
      <div class="layout-choice-row lyrics-text-effect-row">
        <button
          type="button"
          aria-label="歌詞陰影"
          aria-pressed={lyrics.textEffect === 'shadow'}
          class:chosen={lyrics.textEffect === 'shadow'}
          onclick={() => onLyricsChange({ textEffect: 'shadow' }, true)}
        >陰影</button>
        <button
          type="button"
          aria-label="歌詞描邊"
          aria-pressed={lyrics.textEffect === 'stroke'}
          class:chosen={lyrics.textEffect === 'stroke'}
          onclick={() => onLyricsChange({ textEffect: 'stroke' }, true)}
        >描邊</button>
        <button
          type="button"
          aria-label="關閉歌詞文字效果"
          aria-pressed={lyrics.textEffect === 'none'}
          class:chosen={lyrics.textEffect === 'none'}
          onclick={() => onLyricsChange({ textEffect: 'none' }, true)}
        >關閉</button>
      </div>
    </div>
    <label class="lyrics-preference-range">
      <span><strong>非目前歌詞透明度</strong><output>{lyrics.inactiveOpacityPercent}%</output></span>
      <input type="range" min="10" max="100" step="1" value={lyrics.inactiveOpacityPercent} aria-label="非目前歌詞透明度" oninput={(event) => onLyricsChange({ inactiveOpacityPercent: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ inactiveOpacityPercent: Number(event.currentTarget.value) }, true)} />
    </label>
    <label class="lyrics-preference-range">
      <span><strong>原文字級</strong><output>{lyrics.primaryFontSizePx}px</output></span>
      <input type="range" min="0" max="50" step="1" value={lyrics.primaryFontSizePx} aria-label="原文字級" oninput={(event) => onLyricsChange({ primaryFontSizePx: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ primaryFontSizePx: Number(event.currentTarget.value) }, true)} />
    </label>
    <label class="lyrics-preference-range">
      <span><strong>譯文與羅馬拼音字級</strong><output>{lyrics.auxiliaryFontSizePx}px</output></span>
      <input type="range" min="0" max="50" step="1" value={lyrics.auxiliaryFontSizePx} aria-label="譯文與羅馬拼音字級" oninput={(event) => onLyricsChange({ auxiliaryFontSizePx: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ auxiliaryFontSizePx: Number(event.currentTarget.value) }, true)} />
    </label>
    <label class="lyrics-preference-range">
      <span><strong>句間距</strong><output>{lyrics.lineGapPx}px</output></span>
      <input type="range" min="0" max="50" step="1" value={lyrics.lineGapPx} aria-label="歌詞句間距" oninput={(event) => onLyricsChange({ lineGapPx: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ lineGapPx: Number(event.currentTarget.value) }, true)} />
    </label>
  </section>
  {/if}
</div>
