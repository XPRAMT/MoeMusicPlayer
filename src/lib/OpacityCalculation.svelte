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
    <div class="opacity-equation" role="img" aria-label={calculation.displayText}>
      {#if calculation.kind === 'ratio'}
        <math aria-hidden="true" display="block">
          <mfrac>
            <mrow><mtext>目標亮度（{calculation.targetPercent}%）</mtext></mrow>
            <mrow><mtext>圖片平均亮度（{calculation.imagePercent}%）</mtext></mrow>
          </mfrac>
        </math>
        <math class="opacity-result" aria-hidden="true"><mrow><mo>=</mo><mtext>透明度（{calculation.opacityPercent}%）</mtext></mrow></math>
      {:else if calculation.kind === 'inverseRatio'}
        <math aria-hidden="true" display="block">
          <mfrac>
            <mrow><mtext>100%</mtext><mo>−</mo><mtext>目標亮度（{calculation.targetPercent}%）</mtext></mrow>
            <mrow><mtext>100%</mtext><mo>−</mo><mtext>圖片平均亮度（{calculation.imagePercent}%）</mtext></mrow>
          </mfrac>
        </math>
        <math class="opacity-result" aria-hidden="true"><mrow><mo>=</mo><mtext>透明度（{calculation.opacityPercent}%）</mtext></mrow></math>
      {:else}
        <span>目標亮度（{calculation.targetPercent}%）</span>
        <math class="opacity-compare-sign" aria-hidden="true"><mo>{calculation.textColor === 'white' ? '≥' : '≤'}</mo></math>
        <span>圖片平均亮度（{calculation.imagePercent}%）</span>
        <span>，不處理</span>
      {/if}
    </div>
  {:else}
    <small>{emptyMessage}</small>
  {/if}
</div>
