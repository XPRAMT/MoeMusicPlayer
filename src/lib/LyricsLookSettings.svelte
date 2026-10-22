<script lang="ts">
  // 工作列歌詞與浮動歌詞共用的外觀設定（ADR 0032）：對齊、字體大小、顏色、歌詞外觀、譯文與譯文大小。
  // 兩邊各自保存，這裡只畫控制項並把變更交回呼叫端。
  import { tr } from './i18n.js';
  import type { LyricsLookSettings } from './ipc';
  import { LYRICS_OPACITY_PERCENT, TASKBAR_LYRICS_FONT_SIZE_PX, TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT } from './taskbar-lyrics-settings.js';

  let {
    settings,
    onChange,
    idPrefix,
    fontAriaLabel,
    colorAriaLabel,
    autoColorHint,
  }: {
    settings: LyricsLookSettings;
    onChange: (patch: Partial<LyricsLookSettings>) => void;
    idPrefix: string;
    fontAriaLabel: string;
    colorAriaLabel: string;
    autoColorHint: string;
  } = $props();
</script>

<div class="cover-corner-setting" role="group" aria-labelledby="{idPrefix}-align-label">
  <span id="{idPrefix}-align-label" class="cover-corner-setting-label"><strong>{tr("對齊")}</strong></span>
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
<label class="lyrics-preference-toggle">
  <input type="checkbox" checked={settings.useCustomColor} onchange={(event) => onChange({ useCustomColor: event.currentTarget.checked })} />
  <span><strong>{tr("自訂顏色")}</strong><small>{autoColorHint}</small></span>
  <input
    class="taskbar-lyrics-color"
    type="color"
    value={settings.customColor}
    disabled={!settings.useCustomColor}
    aria-label={colorAriaLabel}
    oninput={(event) => onChange({ customColor: event.currentTarget.value })}
  />
</label>
<div class="cover-corner-setting" role="group" aria-labelledby="{idPrefix}-effect-label">
  <span id="{idPrefix}-effect-label" class="cover-corner-setting-label"><strong>{tr("歌詞外觀")}</strong></span>
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
<label class="lyrics-preference-toggle">
  <input type="checkbox" checked={settings.showTranslation} onchange={(event) => onChange({ showTranslation: event.currentTarget.checked })} />
  <span><strong>{tr("顯示譯文")}</strong><small>{tr("歌詞有翻譯時，在原文下方多顯示一行較小的譯文")}</small></span>
</label>
<label class="lyrics-preference-range">
  <span><strong>{tr("譯文大小（相對原文）")}</strong><output>{settings.translationScalePercent}%</output></span>
  <input
    type="range"
    min={TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.min}
    max={TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.max}
    step="1"
    value={settings.translationScalePercent}
    disabled={!settings.showTranslation}
    aria-label={tr("譯文大小（相對原文）")}
    oninput={(event) => onChange({ translationScalePercent: Number(event.currentTarget.value) })}
  />
</label>
<label class="lyrics-preference-range">
  <span><strong>{tr("透明度")}</strong><output>{settings.opacityPercent}%</output></span>
  <input
    type="range"
    min={LYRICS_OPACITY_PERCENT.min}
    max={LYRICS_OPACITY_PERCENT.max}
    step="1"
    value={settings.opacityPercent}
    aria-label={tr("透明度")}
    oninput={(event) => onChange({ opacityPercent: Number(event.currentTarget.value) })}
  />
</label>