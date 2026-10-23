<script lang="ts" module>
  let nextLyricsIdPrefix = 0;
</script>

<script lang="ts">
  import { tr } from './i18n.js';
  import type { BackdropOpacityCalculation, LyricsPreferences, NowPlayingAppearancePreferences, NowPlayingLayout } from './ipc';
  import NowPlayingLayoutSwitch from './NowPlayingLayoutSwitch.svelte';
  import OpacityCalculation from './OpacityCalculation.svelte';
  import LyricsLookSettings from './LyricsLookSettings.svelte';


  interface Props {
    layout: NowPlayingLayout;
    appearance: NowPlayingAppearancePreferences;
    autoBackgroundOpacity?: boolean;
    coverClarityPercent?: number;
    onCoverClarityChange?: (value: number, flush?: boolean) => void;
    opacityCalculation?: BackdropOpacityCalculation | null;
    lyrics: LyricsPreferences;
    onLayoutChange: (layout: NowPlayingLayout) => void;
    onAppearanceChange: (patch: Partial<NowPlayingAppearancePreferences>, flush?: boolean) => void;
    onLyricsChange: (patch: Partial<LyricsPreferences>, flush?: boolean) => void;
    onLyricsReset: () => void;
    groups?: 'playback' | 'lyrics' | 'both';
    showGroupHeading?: boolean;
  }

  let {
    layout, appearance, lyrics,
    autoBackgroundOpacity = false,
    coverClarityPercent = 0,
    onCoverClarityChange = () => {},
    opacityCalculation = null,
    onLayoutChange, onAppearanceChange, onLyricsChange, onLyricsReset,
    groups = 'both',
    showGroupHeading = true,
  }: Props = $props();
  const lyricsIdPrefix = `lyrics-general-${nextLyricsIdPrefix++}`;
</script>

<div class="quick-settings-controls">
  {#if groups === 'playback' || groups === 'both'}
  <section class="quick-settings-group" aria-label={showGroupHeading ? undefined : tr('封面與歌詞排列')} aria-labelledby={showGroupHeading ? 'quick-settings-playback-heading' : undefined}>
    {#if showGroupHeading}
      <h3 id="quick-settings-playback-heading">{tr("播放頁")}</h3>
    {/if}
    <NowPlayingLayoutSwitch {layout} onChange={onLayoutChange} />
    <div class="cover-corner-setting" role="group" aria-labelledby="cover-corner-style-label">
      <span id="cover-corner-style-label" class="cover-corner-setting-label"><strong>{tr("專輯封面造型")}</strong></span>
      <div class="layout-choice-row cover-corner-choice-row">
        <button
          type="button"
          aria-label={tr("專輯封面圓角")}
          aria-pressed={appearance.coverCornerStyle === 'rounded'}
          class:chosen={appearance.coverCornerStyle === 'rounded'}
          onclick={() => onAppearanceChange({ coverCornerStyle: 'rounded' }, true)}
        >
          <strong>{tr("圓角")}</strong><span>{tr("專輯封面使用圓角")}</span>
        </button>
        <button
          type="button"
          aria-label={tr("專輯封面方形")}
          aria-pressed={appearance.coverCornerStyle === 'square'}
          class:chosen={appearance.coverCornerStyle === 'square'}
          onclick={() => onAppearanceChange({ coverCornerStyle: 'square' }, true)}
        >
          <strong>{tr("方形")}</strong><span>{tr("專輯封面不圓角")}</span>
        </button>
      </div>
    </div>
    <label class="lyrics-preference-range">
      <span><strong>{tr("封面背景模糊")}</strong><output>{appearance.backgroundBlurPx}px</output></span>
      <input type="range" min="0" max="50" step="1" value={appearance.backgroundBlurPx} aria-label={tr("封面背景模糊程度")} oninput={(event) => onAppearanceChange({ backgroundBlurPx: Number(event.currentTarget.value) })} onchange={(event) => onAppearanceChange({ backgroundBlurPx: Number(event.currentTarget.value) }, true)} />
    </label>
    {#if autoBackgroundOpacity}
      <label class="lyrics-preference-range">
        <span><strong>{tr("清晰度")}</strong><output>{coverClarityPercent}%</output></span>
        <input type="range" min="0" max="100" step="1" value={coverClarityPercent} aria-label={tr("封面背景清晰度")} oninput={(event) => onCoverClarityChange(Number(event.currentTarget.value))} onchange={(event) => onCoverClarityChange(Number(event.currentTarget.value), true)} />
      </label>
    {/if}
    <label class="lyrics-preference-range">
      <span><strong>{tr("封面背景透明度")}</strong>{#if !autoBackgroundOpacity}<output>{appearance.backgroundOpacityPercent}%</output>{/if}</span>
      {#if autoBackgroundOpacity}
        <OpacityCalculation calculation={opacityCalculation} description={tr("依目前封面圖片的平均亮度與文字顏色自動計算。")} emptyMessage={tr("沒有可讀取的封面圖片，使用固定底色。")} />
      {:else}
        <input type="range" min="0" max="100" step="1" value={appearance.backgroundOpacityPercent} aria-label={tr("封面背景透明度")} oninput={(event) => onAppearanceChange({ backgroundOpacityPercent: Number(event.currentTarget.value) })} onchange={(event) => onAppearanceChange({ backgroundOpacityPercent: Number(event.currentTarget.value) }, true)} />
      {/if}
    </label>
  </section>
  {/if}

  {#if groups === 'lyrics' || groups === 'both'}
  <section class="quick-settings-group" aria-label={showGroupHeading ? undefined : tr('歌詞顯示')} aria-labelledby={showGroupHeading ? 'quick-settings-lyrics-heading' : undefined}>
    {#if showGroupHeading}
      <div class="quick-settings-group-heading">
        <h3 id="quick-settings-lyrics-heading">{tr("歌詞外觀")}</h3>
        <button class="outline-button" type="button" onclick={onLyricsReset}>{tr("恢復預設")}</button>
      </div>
    {/if}
    <LyricsLookSettings
      settings={{
        showTranslation: lyrics.showTranslation,
        fontSizePx: lyrics.primaryFontSizePx,
        translationScalePercent: lyrics.auxiliaryScalePercent,
        textEffect: lyrics.textEffect,
      }}
      onChange={(patch) => onLyricsChange({
        ...(patch.showTranslation !== undefined ? { showTranslation: patch.showTranslation } : {}),
        ...(patch.fontSizePx !== undefined ? { primaryFontSizePx: patch.fontSizePx } : {}),
        ...(patch.translationScalePercent !== undefined ? { auxiliaryScalePercent: patch.translationScalePercent } : {}),
        ...(patch.textEffect !== undefined ? { textEffect: patch.textEffect } : {}),
      })}
      idPrefix={lyricsIdPrefix}
      fontAriaLabel={tr("歌詞字體大小")}
      translationAriaLabel={tr("譯文與羅馬拼音大小（相對原文）")}
      translationActive={lyrics.showTranslation || lyrics.showRomanization}
      withAlign={false}
      withColor={false}
      withOpacity={false}
      withAnimation={false}
    >
      {#snippet displayExtra()}
        <label class="lyrics-preference-toggle">
          <input type="checkbox" checked={lyrics.showRomanization} onchange={(event) => onLyricsChange({ showRomanization: event.currentTarget.checked }, true)} />
          <span><strong>{tr("顯示羅馬拼音")}</strong><small>{tr("在每行原文下方顯示拼音")}</small></span>
        </label>
        <label class="lyrics-preference-toggle">
          <input type="checkbox" checked={lyrics.simplifiedToTraditional} onchange={(event) => onLyricsChange({ simplifiedToTraditional: event.currentTarget.checked }, true)} />
          <span><strong>{tr("簡體轉繁體")}</strong><small>{tr("播放時把整首歌詞轉成繁體字形，不改用詞，也不另存檔案")}</small></span>
        </label>
      {/snippet}
      {#snippet sizeExtra()}
        <label class="lyrics-preference-range">
          <span><strong>{tr("句間距")}</strong><output>{lyrics.lineGapPx}px</output></span>
          <input type="range" min="0" max="50" step="1" value={lyrics.lineGapPx} aria-label={tr("歌詞句間距")} oninput={(event) => onLyricsChange({ lineGapPx: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ lineGapPx: Number(event.currentTarget.value) }, true)} />
        </label>
      {/snippet}
      {#snippet lookExtra()}
        <label class="lyrics-preference-range">
          <span><strong>{tr("非目前歌詞透明度")}</strong><output>{lyrics.inactiveOpacityPercent}%</output></span>
          <input type="range" min="10" max="100" step="1" value={lyrics.inactiveOpacityPercent} aria-label={tr("非目前歌詞透明度")} oninput={(event) => onLyricsChange({ inactiveOpacityPercent: Number(event.currentTarget.value) })} onchange={(event) => onLyricsChange({ inactiveOpacityPercent: Number(event.currentTarget.value) }, true)} />
        </label>
      {/snippet}
    </LyricsLookSettings>
  </section>
  {/if}
</div>
