<script lang="ts">
  // 三個歌詞設定分頁（播放頁／工作列歌詞／浮動歌詞，ADR 0032）共用的版面：
  // 同樣的分組（顯示 → 字體與大小 → 外觀 → 位置與面板）、同樣的順序與標籤。
  // 各頁各自保存，這裡只畫控制項並把變更交回呼叫端；只有某頁才有的設定以 snippet 放進對應分組。
  import type { Snippet } from 'svelte';
  import { tr } from './i18n.js';
  import type { LyricsLookSettings } from './ipc';
  import { LYRICS_FONT_WEIGHT } from './lyrics-preferences.js';
  import { LYRICS_MAIN_OPACITY_PERCENT, LYRICS_SECONDARY_OPACITY_PERCENT, TASKBAR_LYRICS_FONT_SIZE_PX, TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT } from './taskbar-lyrics-settings.js';

  type LookFields = Pick<LyricsLookSettings, 'showTranslation' | 'fontSizePx' | 'translationScalePercent' | 'textEffect' | 'mainOpacityPercent' | 'secondaryOpacityPercent' | 'fontWeight'>
    & Partial<Pick<LyricsLookSettings, 'align' | 'useCustomColor' | 'customColor' | 'animation'>>;

  let {
    settings,
    onChange,
    idPrefix,
    fontAriaLabel,
    colorAriaLabel = '',
    autoColorHint = '',
    alignLabel,
    translationAriaLabel,
    withAlign = true,
    withColor = true,
    withAnimation = true,
    enable,
    displayExtra,
    sizeExtra,
    lookExtra,
    position,
  }: {
    settings: LookFields;
    onChange: (patch: Partial<LyricsLookSettings>) => void;
    idPrefix: string;
    fontAriaLabel: string;
    colorAriaLabel?: string;
    autoColorHint?: string;
    /** Label of the alignment row (floating lyrics: 「文字排版」, the text inside the panel). */
    alignLabel?: string;
    translationAriaLabel?: string;
    withAlign?: boolean;
    withColor?: boolean;
    withAnimation?: boolean;
    /** 「啟用」 toggle, first in 顯示. */
    enable?: Snippet;
    /** Page-only display settings, after 顯示譯文 and before 動畫. */
    displayExtra?: Snippet;
    /** Page-only size settings, after 字體粗細. */
    sizeExtra?: Snippet;
    /** Page-only appearance settings, after 次要透明度 and before 對齊. */
    lookExtra?: Snippet;
    /** 位置與面板 group content (page-specific). */
    position?: Snippet;
  } = $props();
</script>

<section class="quick-settings-group" aria-labelledby="{idPrefix}-display-heading">
  <h3 id="{idPrefix}-display-heading">{tr("顯示")}</h3>
  {@render enable?.()}
  <label class="lyrics-preference-toggle">
    <input type="checkbox" checked={settings.showTranslation} onchange={(event) => onChange({ showTranslation: event.currentTarget.checked })} />
    <span><strong>{tr("顯示譯文")}</strong><small>{tr("在每行原文下方顯示翻譯")}</small></span>
  </label>
  {@render displayExtra?.()}
  {#if withAnimation}
    <div class="cover-corner-setting" role="group" aria-labelledby="{idPrefix}-animation-label">
      <span id="{idPrefix}-animation-label" class="cover-corner-setting-label"><strong>{tr("動畫")}</strong></span>
      <div class="layout-choice-row lyrics-text-effect-row">
        {#each [['none', tr('無'), tr('一次顯示一行，換行時直接切換')], ['continuous', tr('連續'), tr('所有歌詞接成一行，隨播放進度連續捲動')]] as [value, label, description] (value)}
          <button
            type="button"
            aria-label={description}
            title={description}
            aria-pressed={settings.animation === value}
            class:chosen={settings.animation === value}
            onclick={() => onChange({ animation: value as LyricsLookSettings['animation'] })}
          >{label}</button>
        {/each}
      </div>
    </div>
  {/if}
</section>

<section class="quick-settings-group" aria-labelledby="{idPrefix}-size-heading">
  <h3 id="{idPrefix}-size-heading">{tr("字體與大小")}</h3>
  <label class="lyrics-preference-range">
    <span><strong>{tr("字體大小")}</strong><output>{settings.fontSizePx} px</output></span>
    <input
      type="range"
      min={TASKBAR_LYRICS_FONT_SIZE_PX.min}
      max={TASKBAR_LYRICS_FONT_SIZE_PX.max}
      step="1"
      value={settings.fontSizePx}
      aria-label={fontAriaLabel}
      oninput={(event) => onChange({ fontSizePx: Number(event.currentTarget.value) })}
    />
  </label>
  <label class="lyrics-preference-range">
    <span><strong>{tr("譯文大小（相對原文）")}</strong><output>{settings.translationScalePercent}%</output></span>
    <input
      type="range"
      min={TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.min}
      max={TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.max}
      step="1"
      value={settings.translationScalePercent}
      aria-label={translationAriaLabel ?? tr("譯文大小（相對原文）")}
      oninput={(event) => onChange({ translationScalePercent: Number(event.currentTarget.value) })}
    />
  </label>
  <label class="lyrics-preference-range">
    <span><strong>{tr("字體粗細")}</strong><output>{settings.fontWeight}</output></span>
    <input
      type="range"
      min={LYRICS_FONT_WEIGHT.min}
      max={LYRICS_FONT_WEIGHT.max}
      step={LYRICS_FONT_WEIGHT.step}
      value={settings.fontWeight}
      aria-label={tr("字體粗細")}
      oninput={(event) => onChange({ fontWeight: Number(event.currentTarget.value) })}
    />
  </label>
  {@render sizeExtra?.()}
</section>

<section class="quick-settings-group" aria-labelledby="{idPrefix}-look-heading">
  <h3 id="{idPrefix}-look-heading">{tr("文字效果")}</h3>
  <!-- The group heading already says 文字效果: the control keeps only an accessible name, no visible caption. -->
  <div class="cover-corner-setting" role="group" aria-label={tr("文字效果")}>
    <div class="layout-choice-row lyrics-text-effect-row">
      {#each [['shadow', tr('陰影'), tr('歌詞陰影')], ['stroke', tr('描邊'), tr('歌詞描邊')], ['none', tr('關閉'), tr('關閉歌詞外觀')]] as [value, label, description] (value)}
        <button
          type="button"
          aria-label={description}
          aria-pressed={settings.textEffect === value}
          class:chosen={settings.textEffect === value}
          onclick={() => onChange({ textEffect: value as LyricsLookSettings['textEffect'] })}
        >{label}</button>
      {/each}
    </div>
  </div>
  {#if withColor}
    <label class="lyrics-preference-toggle">
      <input type="checkbox" checked={settings.useCustomColor} onchange={(event) => onChange({ useCustomColor: event.currentTarget.checked })} />
      <span><strong>{tr("自訂顏色")}</strong><small>{autoColorHint}</small></span>
      <input
        class="taskbar-lyrics-color"
        type="color"
        value={settings.customColor}
        aria-label={colorAriaLabel}
        oninput={(event) => onChange({ customColor: event.currentTarget.value })}
      />
    </label>
  {/if}
  <label class="lyrics-preference-range">
    <span><strong>{tr("主要透明度")}</strong><output>{settings.mainOpacityPercent}%</output></span>
    <input
      type="range"
      min={LYRICS_MAIN_OPACITY_PERCENT.min}
      max={LYRICS_MAIN_OPACITY_PERCENT.max}
      step="1"
      value={settings.mainOpacityPercent}
      aria-label={tr("主要透明度")}
      oninput={(event) => onChange({ mainOpacityPercent: Number(event.currentTarget.value) })}
    />
  </label>
  <label class="lyrics-preference-range">
    <span><strong>{tr("次要透明度")}</strong><output>{settings.secondaryOpacityPercent}%</output></span>
    <input
      type="range"
      min={LYRICS_SECONDARY_OPACITY_PERCENT.min}
      max={LYRICS_SECONDARY_OPACITY_PERCENT.max}
      step="1"
      value={settings.secondaryOpacityPercent}
      aria-label={tr("次要透明度")}
      oninput={(event) => onChange({ secondaryOpacityPercent: Number(event.currentTarget.value) })}
    />
  </label>
  {@render lookExtra?.()}
  {#if withAlign}
    <div class="cover-corner-setting" role="group" aria-labelledby="{idPrefix}-align-label">
      <span id="{idPrefix}-align-label" class="cover-corner-setting-label"><strong>{alignLabel ?? tr("對齊")}</strong></span>
      <div class="layout-choice-row lyrics-text-effect-row">
        {#each [['left', tr('靠左')], ['center', tr('置中')], ['right', tr('靠右')]] as [value, label] (value)}
          <button
            type="button"
            aria-pressed={settings.align === value}
            class:chosen={settings.align === value}
            onclick={() => onChange({ align: value as LyricsLookSettings['align'] })}
          >{label}</button>
        {/each}
      </div>
    </div>
  {/if}
</section>

{#if position}
  <section class="quick-settings-group" aria-labelledby="{idPrefix}-position-heading">
    <h3 id="{idPrefix}-position-heading">{tr("位置與面板")}</h3>
    {@render position()}
  </section>
{/if}
