<script lang="ts">
  // Daily columns stacked by series (≤24px wide, 4px rounded top, 2px surface gap between
  // segments) with a hover tooltip. Used for short periods where a calendar would be sparse.
  import { scaleLinear } from 'd3-scale'
  import { fmtDate } from '../../lib/i18n.svelte'

  type Series = { key: string; label: string; color: string }
  let {
    dates,
    values,
    series,
    format,
    height = 260,
    ariaLabel,
    selected = null,
    onselect,
  }: {
    dates: string[]
    values: Record<string, number>[]
    series: Series[]
    format: (v: number) => string
    height?: number
    ariaLabel: string
    selected?: string | null
    onselect?: (date: string) => void
  } = $props()

  let width = $state(600)
  let hover: number | null = $state(null)
  const m = { top: 12, right: 8, bottom: 26, left: 52 }
  const iw = $derived(Math.max(10, width - m.left - m.right))
  const ih = $derived(height - m.top - m.bottom)
  const totals = $derived(values.map((v) => series.reduce((a, s) => a + (v[s.key] ?? 0), 0)))
  const y = $derived(scaleLinear().domain([0, Math.max(1e-9, ...totals)]).nice(4).range([ih, 0]))
  const ticks = $derived(y.ticks(4))
  const band = $derived(iw / Math.max(1, dates.length))
  const bw = $derived(Math.max(3, Math.min(24, band * 0.62)))
  const GAP = 2

  // stacked segments, bottom-up, with a surface gap between them
  const stacks = $derived(
    values.map((v) => {
      let acc = 0
      return series
        .map((s) => {
          const val = v[s.key] ?? 0
          const y0 = acc
          acc += val
          return { key: s.key, color: s.color, y0, y1: acc, val }
        })
        .filter((seg) => seg.val > 0)
    }),
  )
  const labelEvery = $derived(Math.max(1, Math.ceil(dates.length / Math.max(2, Math.floor(iw / 70)))))

  function barPath(x0: number, top: number, bottom: number, w: number, round: boolean): string {
    const h = Math.max(0, bottom - top)
    const r = round ? Math.min(4, w / 2, h) : 0
    return `M${x0},${bottom}V${top + r}${r ? `Q${x0},${top} ${x0 + r},${top}` : ''}H${x0 + w - r}${r ? `Q${x0 + w},${top} ${x0 + w},${top + r}` : ''}V${bottom}Z`
  }
</script>

<div class="wrap" bind:clientWidth={width}>
  {#if series.length > 1}
    <div class="legend" aria-hidden="true">
      {#each series as s (s.key)}<span><i style="background:{s.color}"></i>{s.label}</span>{/each}
    </div>
  {/if}
  <svg {width} {height} role="img" aria-label={ariaLabel}>
    <g transform="translate({m.left},{m.top})">
      {#each ticks as tk (tk)}
        <line class="grid" x1="0" x2={iw} y1={y(tk)} y2={y(tk)} />
        <text class="axis" x="-8" y={y(tk)} dy="0.32em" text-anchor="end">{format(tk)}</text>
      {/each}
      {#each stacks as segs, i (dates[i])}
        {@const cx = band * i + band / 2}
        <g class="col" class:dim={hover !== null && hover !== i} class:sel={selected === dates[i]}>
          {#each segs as seg, si (seg.key)}
            {@const top = y(seg.y1) + (si < segs.length - 1 ? GAP / 2 : 0)}
            {@const bottom = y(seg.y0) - (si > 0 ? GAP / 2 : 0)}
            <path d={barPath(cx - bw / 2, top, bottom, bw, si === segs.length - 1)} fill={seg.color} />
          {/each}
        </g>
        <!-- generous hit target: the whole band -->
        <rect
          class="hit"
          x={band * i}
          y="0"
          width={band}
          height={ih}
          role="presentation"
          onpointerenter={() => (hover = i)}
          onpointerleave={() => (hover = null)}
          onclick={() => onselect?.(dates[i])}
        />
        {#if i % labelEvery === 0 || i === dates.length - 1}
          <text class="axis" x={cx} y={ih + 18} text-anchor="middle">{fmtDate(dates[i], 'short')}</text>
        {/if}
      {/each}
      <line class="base" x1="0" x2={iw} y1={ih} y2={ih} />
    </g>
  </svg>
  {#if hover !== null}
    {@const left = m.left + band * hover + band / 2}
    <div class="tip" style="left:{Math.min(Math.max(left, 90), width - 90)}px">
      <div class="tip-date">{fmtDate(dates[hover], 'long')}</div>
      {#each [...series].reverse() as s (s.key)}
        <div class="tip-row"><i style="background:{s.color}"></i><span>{s.label}</span><b class="num">{format(values[hover]?.[s.key] ?? 0)}</b></div>
      {/each}
      {#if series.length > 1}
        <div class="tip-row total"><span>Σ</span><b class="num">{format(totals[hover])}</b></div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    width: 100%;
  }
  svg {
    display: block;
    overflow: visible;
  }
  .grid {
    stroke: var(--grid);
  }
  .base {
    stroke: var(--axis);
  }
  .axis {
    fill: var(--ink-3);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .col {
    transition: opacity var(--dur) var(--ease);
  }
  .col.dim {
    opacity: 0.45;
  }
  .col.sel path {
    stroke: var(--ink);
    stroke-width: 1.5;
  }
  .hit {
    fill: transparent;
  }
  .legend {
    display: flex;
    gap: 14px;
    font-size: 12px;
    color: var(--ink-2);
    margin-bottom: 8px;
  }
  .legend span,
  .tip-row {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .legend i,
  .tip-row i {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    display: inline-block;
  }
  .tip {
    position: absolute;
    top: 6px;
    transform: translateX(-50%);
    pointer-events: none;
    background: var(--surface);
    border: 0.5px solid var(--hairline-strong);
    box-shadow: var(--shadow-pop);
    border-radius: 10px;
    padding: 8px 10px;
    font-size: 12px;
    min-width: 170px;
    z-index: 5;
  }
  .tip-date {
    color: var(--ink-2);
    margin-bottom: 4px;
  }
  .tip-row {
    display: flex;
    width: 100%;
  }
  .tip-row span {
    flex: 1;
    color: var(--ink-2);
  }
  .tip-row.total {
    border-top: 0.5px solid var(--hairline);
    margin-top: 4px;
    padding-top: 4px;
  }
</style>
