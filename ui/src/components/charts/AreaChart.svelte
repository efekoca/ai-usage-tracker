<script lang="ts">
  // Daily series as overlaid 2px lines with light area washes and a crosshair tooltip.
  import { area, line, curveMonotoneX } from 'd3-shape'
  import { scaleLinear } from 'd3-scale'
  import { fmtDate } from '../../lib/i18n.svelte'

  type Series = { key: string; label: string; color: string }
  let {
    dates,
    values,
    series,
    format,
    height = 220,
    ariaLabel,
  }: {
    dates: string[]
    values: Record<string, number>[]
    series: Series[]
    format: (v: number) => string
    height?: number
    ariaLabel: string
  } = $props()

  let width = $state(600)
  let hover: number | null = $state(null)
  const m = { top: 12, right: 12, bottom: 26, left: 52 }
  const iw = $derived(Math.max(10, width - m.left - m.right))
  const ih = $derived(height - m.top - m.bottom)

  // series are overlaid, not stacked, so every line reads as its own value
  const layers = $derived(series.map((s) => values.map((v) => [0, v[s.key] ?? 0] as [number, number])))
  const maxY = $derived(Math.max(1e-9, ...values.flatMap((v) => series.map((s) => v[s.key] ?? 0)).filter(Number.isFinite)))
  const x = $derived(scaleLinear().domain([0, Math.max(1, dates.length - 1)]).range([0, iw]))
  const y = $derived(scaleLinear().domain([0, maxY]).nice(4).range([ih, 0]))
  const ticks = $derived(y.ticks(4))

  const areaGen = $derived(
    area<[number, number]>()
      .defined((d) => Number.isFinite(d[1]))
      .x((_, i) => x(i))
      .y0((d) => y(d[0]))
      .y1((d) => y(d[1]))
      .curve(curveMonotoneX),
  )
  const lineGen = $derived(
    line<[number, number]>()
      .defined((d) => Number.isFinite(d[1]))
      .x((_, i) => x(i))
      .y((d) => y(d[1]))
      .curve(curveMonotoneX),
  )

  // ~6 evenly spaced date labels
  const xLabels = $derived.by(() => {
    const n = dates.length
    if (n === 0) return []
    const step = Math.max(1, Math.ceil(n / Math.max(2, Math.floor(iw / 90))))
    const out: number[] = []
    for (let i = 0; i < n; i += step) out.push(i)
    if (out[out.length - 1] !== n - 1 && n > 1 && (n - 1 - out[out.length - 1]) * (iw / n) > 50) out.push(n - 1)
    return out
  })

  function move(e: PointerEvent) {
    const r = (e.currentTarget as SVGRectElement).getBoundingClientRect()
    const i = Math.round(x.invert(e.clientX - r.left))
    hover = Math.max(0, Math.min(dates.length - 1, i))
  }


  const total = (i: number) => series.reduce((a, s) => a + (values[i]?.[s.key] ?? 0), 0)
</script>

<div class="wrap" bind:clientWidth={width}>
  {#if series.length > 1}
    <div class="legend" aria-hidden="true">
      {#each series as s (s.key)}
        <span><i style="background:{s.color}"></i>{s.label}</span>
      {/each}
    </div>
  {/if}
  <svg {width} {height} role="img" aria-label={ariaLabel}>
    <g transform="translate({m.left},{m.top})">
      {#each ticks as tk (tk)}
        <line class="grid" x1="0" x2={iw} y1={y(tk)} y2={y(tk)} />
        <text class="axis" x="-8" y={y(tk)} dy="0.32em" text-anchor="end">{format(tk)}</text>
      {/each}
      {#each layers as layer, li (series[li].key)}
        <path d={areaGen(layer) ?? ''} fill={series[li].color} fill-opacity="0.1" />
        <path d={lineGen(layer) ?? ''} fill="none" stroke={series[li].color} stroke-width="2" stroke-linejoin="round" stroke-linecap="round" />
      {/each}
      <line class="base" x1="0" x2={iw} y1={ih} y2={ih} />
      {#each xLabels as i (i)}
        <text class="axis" x={x(i)} y={ih + 18} text-anchor={i === 0 ? 'start' : i === dates.length - 1 ? 'end' : 'middle'}>{fmtDate(dates[i], 'short')}</text>
      {/each}
      {#if hover !== null}
        <line class="cross" x1={x(hover)} x2={x(hover)} y1="0" y2={ih} />
        {#each layers as layer, li (series[li].key)}
          {#if Number.isFinite(layer[hover][1])}
          <circle cx={x(hover)} cy={y(layer[hover][1])} r="4.5" fill={series[li].color} stroke="var(--surface)" stroke-width="2" />
          {/if}
        {/each}
      {/if}
      <rect class="hit" width={iw} height={ih} role="presentation" onpointermove={move} onpointerleave={() => (hover = null)} />
    </g>
  </svg>
  {#if hover !== null}
    {@const left = m.left + x(hover)}
    <div class="tip" style="left:{Math.min(Math.max(left, 90), width - 90)}px">
      <div class="tip-date">{fmtDate(dates[hover], 'long')}</div>
      {#each [...series].reverse() as s (s.key)}
        <div class="tip-row"><i style="background:{s.color}"></i><span>{s.label}</span><b class="num">{Number.isFinite(values[hover]?.[s.key] ?? 0) ? format(values[hover]?.[s.key] ?? 0) : '—'}</b></div>
      {/each}
      {#if series.length > 1}
        <div class="tip-row total"><span>Σ</span><b class="num">{format(total(hover))}</b></div>
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
  svg:focus-visible {
    outline-offset: 4px;
  }
  .grid {
    stroke: var(--grid);
    stroke-width: 1;
  }
  .base {
    stroke: var(--axis);
    stroke-width: 1;
  }
  .axis {
    fill: var(--ink-3);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .cross {
    stroke: var(--ink-3);
    stroke-width: 1;
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
    height: 3px;
    border-radius: 2px;
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
