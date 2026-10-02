<script lang="ts">
  // Categorical columns (one series): ≤40px wide, 4px rounded top anchored to the baseline,
  // value labels above, hover tooltip with the full detail. Used for size distributions.
  import { scaleLinear } from 'd3-scale'

  type Bar = { key: string; label: string; value: number; detail?: string }
  let {
    bars,
    format,
    color = 'var(--s3)',
    height = 220,
    ariaLabel,
  }: { bars: Bar[]; format: (v: number) => string; color?: string; height?: number; ariaLabel: string } = $props()

  let width = $state(600)
  let hover: number | null = $state(null)
  const m = { top: 22, right: 8, bottom: 30, left: 8 }
  const iw = $derived(Math.max(10, width - m.left - m.right))
  const ih = $derived(height - m.top - m.bottom)
  const y = $derived(scaleLinear().domain([0, Math.max(1e-9, ...bars.map((b) => b.value))]).range([ih, 0]))
  const band = $derived(iw / Math.max(1, bars.length))
  const bw = $derived(Math.max(6, Math.min(40, band * 0.6)))
  const R = 4

  // rounded top only; a zero bar draws nothing
  function bar(x: number, top: number, w: number, h: number) {
    if (h <= 0) return ''
    const r = Math.min(R, h, w / 2)
    return `M${x},${top + h}V${top + r}Q${x},${top} ${x + r},${top}H${x + w - r}Q${x + w},${top} ${x + w},${top + r}V${top + h}Z`
  }
</script>

<div class="wrap" bind:clientWidth={width}>
  <svg {width} {height} role="img" aria-label={ariaLabel}>
    <g transform="translate({m.left},{m.top})">
      <line x1="0" x2={iw} y1={ih} y2={ih} class="base" />
      {#each bars as b, i (b.key)}
        {@const cx = i * band + band / 2}
        {@const h = ih - y(b.value)}
        <g
          role="presentation"
          onmouseenter={() => (hover = i)}
          onmouseleave={() => (hover = null)}
        >
          <rect x={i * band} y="0" width={band} height={ih + m.bottom} fill="transparent" />
          <path d={bar(cx - bw / 2, y(b.value), bw, h)} fill={color} opacity={hover === null || hover === i ? 1 : 0.55} />
          {#if b.value > 0}
            <text x={cx} y={y(b.value) - 6} text-anchor="middle" class="val">{format(b.value)}</text>
          {/if}
          <text x={cx} y={ih + 18} text-anchor="middle" class="lab">{b.label}</text>
        </g>
      {/each}
    </g>
  </svg>
  {#if hover !== null && bars[hover]}
    {@const b = bars[hover]}
    <div class="tip" style="left:{Math.min(width - 180, Math.max(0, m.left + hover * band + band / 2 - 90))}px">
      <b>{b.label}</b>
      <span>{format(b.value)}</span>
      {#if b.detail}<span class="subtle">{b.detail}</span>{/if}
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
  .base {
    stroke: var(--hairline-strong);
  }
  .val {
    font-size: 11px;
    fill: var(--ink-2);
    font-variant-numeric: tabular-nums;
  }
  .lab {
    font-size: 11px;
    fill: var(--ink-2);
  }
  .tip {
    position: absolute;
    top: 0;
    width: 180px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--surface-raised, var(--surface));
    border: 0.5px solid var(--hairline-strong);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.16);
    font-size: 12px;
    pointer-events: none;
  }
</style>
