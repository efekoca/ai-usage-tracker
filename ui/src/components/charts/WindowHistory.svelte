<script lang="ts">
  // Limit windows on a time axis: each bar spans a window (start to reset) and is as tall as
  // the highest reading in it. Filled windows use the critical status color (with a label in
  // the legend and tooltip); windows not watched to their end are hatched ("at least").
  import { scaleLinear } from 'd3-scale'
  import type { WindowRecord } from '../../lib/api'
  import { fmtDate, fmtDateTime, fmtDuration, fmtPct, t } from '../../lib/i18n.svelte'

  let { windows, fromMs, toMs, ariaLabel, height = 150 }: { windows: WindowRecord[]; fromMs: number; toMs: number; ariaLabel: string; height?: number } = $props()

  let width = $state(480)
  let hover: number | null = $state(null)
  const uid = `h${Math.random().toString(36).slice(2, 8)}`
  const m = { top: 10, right: 6, bottom: 22, left: 34 }
  const iw = $derived(Math.max(10, width - m.left - m.right))
  const ih = $derived(height - m.top - m.bottom)
  const x = $derived(scaleLinear().domain([fromMs, toMs]).range([0, iw]).clamp(true))
  const y = $derived(scaleLinear().domain([0, 100]).range([ih, 0]))
  const shown = $derived(windows.filter((w) => (w.resets_at_ms ?? w.last_ms) >= fromMs))
  const endOf = (w: WindowRecord) => w.end_ms ?? w.resets_at_ms ?? Math.max(w.last_ms, (w.start_ms ?? w.first_ms) + 60_000)
  const bars = $derived(
    [...shown].sort((a, b) => (a.start_ms ?? a.first_ms) - (b.start_ms ?? b.first_ms)).map((w, i, all) => {
      // a window starts no earlier than the one before it ended (a reset cannot overlap)
      const prevEnd = i > 0 ? endOf(all[i - 1]) : -Infinity
      const start = Math.max(w.start_ms ?? w.first_ms, Math.min(prevEnd, w.first_ms))
      const end = endOf(w)
      const x0 = x(start)
      const x1 = x(Math.min(end, toMs))
      const bw = Math.max(3, x1 - x0 - 1)
      return { w, x0: x1 - x0 < 3 ? (x0 + x1) / 2 - 1.5 : x0, bw, top: y(w.peak_pct) }
    }),
  )
  const kind = (w: WindowRecord) => (w.full ? 'full' : w.in_progress ? 'running' : w.complete ? 'complete' : 'partial')
  const ticks = $derived.by(() => {
    const n = Math.max(2, Math.min(5, Math.floor(iw / 110)))
    return Array.from({ length: n + 1 }, (_, i) => fromMs + ((toMs - fromMs) * i) / n)
  })

  function bar(x0: number, top: number, w: number): string {
    const h = Math.max(1, ih - top)
    const r = Math.min(4, w / 2, h)
    return `M${x0},${ih}V${ih - h + r}Q${x0},${ih - h} ${x0 + r},${ih - h}H${x0 + w - r}Q${x0 + w},${ih - h} ${x0 + w},${ih - h + r}V${ih}Z`
  }
</script>

<div class="wrap" bind:clientWidth={width}>
  <svg {width} {height} role="img" aria-label={ariaLabel}>
    <defs>
      <pattern id="{uid}-hatch" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
        <rect width="5" height="5" fill="var(--surface)" />
        <line x1="0" y1="0" x2="0" y2="5" stroke="var(--s1)" stroke-width="2" />
      </pattern>
    </defs>
    <g transform="translate({m.left},{m.top})">
      {#each [0, 50, 100] as v (v)}
        <line class:grid={v !== 100} class:full-line={v === 100} x1="0" x2={iw} y1={y(v)} y2={y(v)} />
        <text class="axis" x="-6" y={y(v)} dy="0.32em" text-anchor="end">{fmtPct(v)}</text>
      {/each}
      {#each bars as b, i (b.w.first_ms + ':' + (b.w.resets_at_ms ?? 0))}
        {@const k = kind(b.w)}
        <path
          class="bar {k}"
          class:dim={hover !== null && hover !== i}
          d={bar(b.x0, b.top, b.bw)}
          fill={k === 'partial' ? `url(#${uid}-hatch)` : k === 'full' ? 'var(--critical)' : 'var(--s1)'}
        />
        <rect class="hit" x={b.x0 - 3} y="0" width={b.bw + 6} height={ih} role="presentation" onpointerenter={() => (hover = i)} onpointerleave={() => (hover = null)} />
      {/each}
      {#each ticks as tk, i (tk)}
        <text class="axis" x={x(tk)} y={ih + 16} text-anchor={i === 0 ? 'start' : i === ticks.length - 1 ? 'end' : 'middle'}>{fmtDate(tk, 'short')}</text>
      {/each}
      <line class="base" x1="0" x2={iw} y1={ih} y2={ih} />
    </g>
  </svg>
  {#if hover !== null && bars[hover]}
    {@const b = bars[hover]}
    {@const w = b.w}
    <div class="tip" style="left:{Math.min(Math.max(m.left + b.x0 + b.bw / 2, 110), width - 110)}px">
      <div class="tip-date">
        {fmtDateTime(w.start_ms ?? w.first_ms)} – {fmtDateTime(w.end_ms ?? w.resets_at_ms ?? w.last_ms)}
      </div>
      <div class="tip-main">{w.complete || w.full ? t('history.tip.peak', { pct: fmtPct(w.peak_pct) }) : t('history.tip.atLeast', { pct: fmtPct(w.peak_pct) })}</div>
      {#if w.full}
        <div>{w.full_minutes !== null ? t('history.tip.full', { d: fmtDuration(w.full_minutes * 60000) }) : t('history.tip.fullOnly')}</div>
      {/if}
      {#if w.in_progress}<div>{t('history.tip.running')}</div>{/if}
      {#if !w.resets_at_ms}<div class="subtle">{t('history.tip.noReset')}</div>{:else if w.end_ms !== null && w.end_ms < w.resets_at_ms}<div class="subtle">{t('history.tip.earlyReset')}</div>{/if}
      <div class="subtle">{t('history.tip.readings', { n: w.readings })}</div>
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    width: 100%;
    min-width: 0;
  }
  svg {
    display: block;
    overflow: visible;
  }
  .grid {
    stroke: var(--grid);
  }
  .full-line {
    stroke: var(--axis);
    stroke-dasharray: 3 3;
  }
  .base {
    stroke: var(--axis);
  }
  .axis {
    fill: var(--ink-3);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .bar {
    transition: opacity var(--dur) var(--ease);
  }
  .bar.partial {
    stroke: var(--s1);
    stroke-width: 1;
  }
  .bar.running {
    opacity: 0.55;
  }
  .bar.dim {
    opacity: 0.35;
  }
  .hit {
    fill: transparent;
  }
  .tip {
    position: absolute;
    top: 4px;
    transform: translateX(-50%);
    pointer-events: none;
    background: var(--surface);
    border: 0.5px solid var(--hairline-strong);
    box-shadow: var(--shadow-pop);
    border-radius: 10px;
    padding: 8px 10px;
    font-size: 12px;
    min-width: 200px;
    z-index: 5;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tip-date {
    color: var(--ink-2);
  }
  .tip-main {
    font-weight: 600;
  }
</style>
