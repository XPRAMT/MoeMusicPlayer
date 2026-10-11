<script lang="ts">
  import type { BackdropOpacityCalculation } from './ipc';

  interface Props {
    calculation: BackdropOpacityCalculation | null;
    description: string;
    emptyMessage: string;
  }

  let { calculation, description, emptyMessage }: Props = $props();
</script>

<div class="opacity-calculation-details">
  <small>{description}</small>
  {#if calculation}
    <math class="opacity-equation" display="block" aria-label={calculation.displayText}>
      {#if calculation.kind === 'ratio'}
        <mrow>
          <mfrac>
            <mrow><mtext>目標亮度（{calculation.targetPercent}%）</mtext></mrow>
            <mrow><mtext>圖片平均亮度（{calculation.imagePercent}%）</mtext></mrow>
          </mfrac>
          <mo>=</mo>
          <mtext>透明度（{calculation.opacityPercent}%）</mtext>
        </mrow>
      {:else if calculation.kind === 'inverseRatio'}
        <mrow>
          <mfrac>
            <mrow><mtext>100%</mtext><mo>−</mo><mtext>目標亮度（{calculation.targetPercent}%）</mtext></mrow>
            <mrow><mtext>100%</mtext><mo>−</mo><mtext>圖片平均亮度（{calculation.imagePercent}%）</mtext></mrow>
          </mfrac>
          <mo>=</mo>
          <mtext>透明度（{calculation.opacityPercent}%）</mtext>
        </mrow>
      {:else}
        <mrow>
          <mtext>目標亮度（{calculation.targetPercent}%）</mtext>
          <mo>{calculation.textColor === 'white' ? '≥' : '≤'}</mo>
          <mtext>圖片平均亮度（{calculation.imagePercent}%）</mtext>
          <mtext>，不處理</mtext>
        </mrow>
      {/if}
    </math>
  {:else}
    <small>{emptyMessage}</small>
  {/if}
</div>
