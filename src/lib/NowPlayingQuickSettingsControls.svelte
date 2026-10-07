<script lang="ts">
  import type { LyricsPreferences, NowPlayingAppearancePreferences, NowPlayingLayout } from './ipc';
  import NowPlayingLayoutSwitch from './NowPlayingLayoutSwitch.svelte';

  interface Props {
    layout: NowPlayingLayout;
    appearance: NowPlayingAppearancePreferences;
    lyrics: LyricsPreferences;
    appearanceState: 'loading' | 'saved' | 'saving' | 'error' | 'preview';
    appearanceError: string | null;
    lyricsState: 'loading' | 'saved' | 'saving' | 'error' | 'preview';
    lyricsError: string | null;
    layoutState: 'loading' | 'saved' | 'saving' | 'error' | 'preview';
    layoutError: string | null;
    onLayoutChange: (layout: NowPlayingLayout) => void;
    onAppearanceChange: (patch: Partial<NowPlayingAppearancePreferences>, flush?: boolean) => void;
    onLyricsChange: (patch: Partial<LyricsPreferences>, flush?: boolean) => void;
    onLyricsReset: () => void;
    groups?: 'playback' | 'lyrics' | 'both';
  }

  let {
    layout, appearance, lyrics, appearanceState, appearanceError, lyricsState, lyricsError, layoutState, layoutError,
    onLayoutChange, onAppearanceChange, onLyricsChange, onLyricsReset, groups = 'both',
  }: Props = $props();
</script>

<div class="quick-settings-controls">
  {#if groups === 'playback' || groups === 'both'}
  <section class="quick-settings-group" aria-labelledby="quick-settings-playback-heading">
    <h3 id="quick-settings-playback-heading">播放頁</h3>
    <NowPlayingLayoutSwitch {layout} onChange={onLayoutChange} />
    <p class="settings-preference-status" class:error={layoutState === 'error'} role="status">
      {layoutError ?? (layoutState === 'loading' ? '正在讀取正在播放排列…' : layoutState === 'saving' ? '正在保存排列…' : layoutState === 'preview' ? '瀏覽器預覽不會保存排列。' : `排列 ${layout.toUpperCase()} 已保存。`)}
    </p>
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
    <div
      class="now-playing-appearance-preview"
      style={`--preview-blur: ${appearance.backgroundBlurPx}px; --preview-background-overlay-alpha: ${(100 - appearance.backgroundBrightnessPercent) / 100};`}
      role="img"
      aria-label={`外觀預覽：模糊 ${appearance.backgroundBlurPx} 像素，封面背景亮度 ${appearance.backgroundBrightnessPercent}%`}
    >
      <span class="now-playing-appearance-preview-surface">播放頁工具列</span>
      <span class="now-playing-appearance-preview-dock">底部播放控制</span>
    </div>
    <label class="lyrics-preference-range">
      <span><strong>封面背景模糊</strong><output>{appearance.backgroundBlurPx}px</output></span>
      <input type="range" min="0" max="40" step="1" value={appearance.backgroundBlurPx} aria-label="封面背景模糊程度" oninput={(event) => onAppearanceChange({ backgroundBlurPx: Number(event.currentTarget.value) })} onchange={(event) => onAppearanceChange({ backgroundBlurPx: Number(event.currentTarget.value) }, true)} />
    </label>
    <label class="lyrics-preference-range">
      <span><strong>封面背景亮度</strong><output>{appearance.backgroundBrightnessPercent}%</output></span>
      <input type="range" min="0" max="100" step="1" value={appearance.backgroundBrightnessPercent} aria-label="封面背景亮度" oninput={(event) => onAppearanceChange({ backgroundBrightnessPercent: Number(event.currentTarget.value) })} onchange={(event) => onAppearanceChange({ backgroundBrightnessPercent: Number(event.currentTarget.value) }, true)} />
    </label>
    <p class="settings-preference-status" class:error={appearanceState === 'error'} role="status">
      {appearanceError ?? (appearanceState === 'loading' ? '正在讀取正在播放外觀…' : appearanceState === 'saving' ? '正在保存正在播放外觀…' : appearanceState === 'preview' ? '瀏覽器預覽不會保存正在播放外觀。' : '正在播放外觀已保存。')}
    </p>
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
      <div class="layout-choice-row cover-corner-choice-row">
        <button
          type="button"
          aria-label="歌詞陰影"
          aria-pressed={lyrics.textEffect === 'shadow'}
          class:chosen={lyrics.textEffect === 'shadow'}
          onclick={() => onLyricsChange({ textEffect: 'shadow' }, true)}
        >
          <strong>陰影</strong><span>文字加上柔和陰影</span>
        </button>
        <button
          type="button"
          aria-label="歌詞描邊"
          aria-pressed={lyrics.textEffect === 'stroke'}
          class:chosen={lyrics.textEffect === 'stroke'}
          onclick={() => onLyricsChange({ textEffect: 'stroke' }, true)}
        >
          <strong>描邊</strong><span>文字加上外框描邊</span>
        </button>
        <button
          type="button"
          aria-label="關閉歌詞文字效果"
          aria-pressed={lyrics.textEffect === 'none'}
          class:chosen={lyrics.textEffect === 'none'}
          onclick={() => onLyricsChange({ textEffect: 'none' }, true)}
        >
          <strong>關閉</strong><span>不描邊也不加陰影</span>
        </button>
      </div>
    </div>
    <label class="lyrics-preference-range">
      <span><strong>非目前歌詞透明度</strong><output>{lyrics.inactiveOpacityPercent}%</output></span>
      <input type="range" min="10" max="100" step="1" value={lyrics.inactiveOpacityPercent} aria-label="非目前歌詞透明度" oninput={(event) => onLyricsChange({ inactiveOpacityPercent: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ inactiveOpacityPercent: Number(event.currentTarget.value) }, true)} />
    </label>
    <label class="lyrics-preference-range">
      <span><strong>原文字級</strong><output>{lyrics.primaryFontSizePx}px</output></span>
      <input type="range" min="12" max="36" step="1" value={lyrics.primaryFontSizePx} aria-label="原文字級" oninput={(event) => onLyricsChange({ primaryFontSizePx: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ primaryFontSizePx: Number(event.currentTarget.value) }, true)} />
    </label>
    <label class="lyrics-preference-range">
      <span><strong>譯文與羅馬拼音字級</strong><output>{lyrics.auxiliaryFontSizePx}px</output></span>
      <input type="range" min="9" max="24" step="1" value={lyrics.auxiliaryFontSizePx} aria-label="譯文與羅馬拼音字級" oninput={(event) => onLyricsChange({ auxiliaryFontSizePx: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ auxiliaryFontSizePx: Number(event.currentTarget.value) }, true)} />
    </label>
    <label class="lyrics-preference-range">
      <span><strong>句間距</strong><output>{lyrics.lineGapPx}px</output></span>
      <input type="range" min="0" max="64" step="1" value={lyrics.lineGapPx} aria-label="歌詞句間距" oninput={(event) => onLyricsChange({ lineGapPx: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ lineGapPx: Number(event.currentTarget.value) }, true)} />
    </label>
    <p class="settings-preference-status" class:error={lyricsState === 'error'} role="status">
      {lyricsError ?? (lyricsState === 'loading' ? '正在讀取歌詞設定…' : lyricsState === 'saving' ? '正在保存歌詞設定…' : lyricsState === 'preview' ? '瀏覽器預覽不會保存歌詞設定。' : '歌詞設定已保存。')}
    </p>
  </section>
  {/if}
</div>
