<script lang="ts">
  import Icon from './Icon.svelte'
  import { fmtDec, fmtPct, t } from '../lib/i18n.svelte'
  import type { Snippet } from 'svelte'

  let {
    label,
    value,
    current = null,
    previous = null,
    hint = '',
    hero = false,
    points = false,
    children,
  }: {
    label: string
    value: string
    current?: number | null
    previous?: number | null
    hint?: string
    hero?: boolean
    /** the value is a percentage: its change is shown in points, as 60 % to 66 % is 6 points and not 10 % */
    points?: boolean
    children?: Snippet
  } = $props()

  const delta = $derived(current === null || previous === null ? null : points ? current - previous : previous > 0 ? (current - previous) / previous : null)
  // a change that prints as zero has no direction to announce
  const flat = $derived(delta === null || (points ? Math.abs(delta) < 0.05 : Math.abs(delta) < 0.005))
</script>

<div class="tile" class:hero>
  <div class="label" title={hint || undefined}>{label}</div>
  <div class="value">{value}</div>
  <div class="foot">
    {#if delta !== null && Number.isFinite(delta)}
      <span class="delta" class:up={!flat && delta > 0} class:down={!flat && delta < 0}>
        {#if !flat}<Icon name={delta > 0 ? 'up' : 'down'} size={12} /><span class="sr-only">{t(delta > 0 ? 'overview.deltaUp' : 'overview.deltaDown')}</span>{/if}
        <!-- beyond +200 % a multiplier reads better than a four-digit percentage -->
        {points ? t('overview.deltaPoints', { pts: fmtDec(Math.abs(delta), 1) }) : delta >= 2 ? `${fmtDec(1 + delta, delta >= 9 ? 0 : 1)}×` : fmtPct(Math.abs(delta) * 100)}
      </span>
      <span class="subtle">{t('overview.vsPrev')}</span>
    {/if}
    {@render children?.()}
  </div>
</div>

<style>
  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .label {
    font-size: 12.5px;
    color: var(--ink-2);
    font-weight: 500;
  }
  .value {
    font-size: 24px;
    font-weight: 650;
    letter-spacing: -0.02em;
    line-height: 1.2;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .hero .value {
    font-size: 34px;
    letter-spacing: -0.03em;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    min-height: 18px;
    flex-wrap: wrap;
  }
  .delta {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-weight: 600;
    color: var(--ink-2);
  }
  /* more usage is neither good nor bad here, so direction is shown by the arrow, not by color */
</style>
