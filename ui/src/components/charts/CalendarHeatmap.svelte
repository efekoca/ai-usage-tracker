<script lang="ts">
  // Year-style calendar: one column per week (Mon→Sun), single-hue sequential ramp.
  import { fmtDate, fmtMonth, weekdayNames } from '../../lib/i18n.svelte'

  let {
    days,
    value,
    format,
    ariaLabel,
    onselect,
    selected = null,
  }: {
    days: { date: string }[]
    value: (i: number) => number
    format: (v: number) => string
    ariaLabel: string
    onselect?: (date: string) => void
    selected?: string | null
  } = $props()

  const CELL = 13
  const GAP = 3
  let hover: number | null = $state(null)

  // offset so that the first column starts on Monday
  const firstDow = $derived(days.length ? (new Date(days[0].date + 'T00:00:00').getDay() + 6) % 7 : 0)
  const cols = $derived(Math.ceil((firstDow + days.length) / 7))
  const pos = (i: number) => ({ c: Math.floor((i + firstDow) / 7), r: (i + firstDow) % 7 })

  // quantile thresholds over non-zero days → 5 levels
  const levels = $derived.by(() => {
    const v = days.map((_, i) => value(i)).filter((x) => x > 0).sort((a, b) => a - b)
    if (!v.length) return [Infinity, Infinity, Infinity, Infinity]
    const q = (p: number) => v[Math.min(v.length - 1, Math.floor(p * v.length))]
    return [q(0.2), q(0.4), q(0.6), q(0.8)]
  })
  function level(x: number) {
    if (x <= 0) return 0
    let l = 1
    for (const th of levels) if (x > th) l++
    return Math.min(5, l)
  }
  const ramp = ['var(--seq-0)', 'var(--seq-2)', 'var(--seq-3)', 'var(--seq-4)', 'var(--seq-5)', 'var(--seq-6)']

  // one label per month, at the first full week column; drop labels that would collide
  const months = $derived.by(() => {
    const out: { c: number; label: string }[] = []
    let last = ''
    days.forEach((d, i) => {
      const m = d.date.slice(0, 7)
      if (m === last) return
      last = m
      const p = pos(i)
      out.push({ c: p.r === 0 ? p.c : p.c + 1, label: fmtMonth(d.date) })
    })
    return out.filter((m, i, a) => m.c < cols && (i === 0 || m.c - a[i - 1].c >= 3))
  })

  const svgW = $derived(36 + cols * (CELL + GAP))
  const svgH = 18 + 7 * (CELL + GAP)
  const names = $derived(weekdayNames())

  function key(e: KeyboardEvent) {
    const h = hover ?? days.length - 1
    const map: Record<string, number> = { ArrowLeft: -7, ArrowRight: 7, ArrowUp: -1, ArrowDown: 1 }
    if (e.key in map) {
      e.preventDefault()
      hover = Math.max(0, Math.min(days.length - 1, h + map[e.key]))
    } else if (e.key === 'Enter' && hover !== null) onselect?.(days[hover].date)
    else if (e.key === 'Escape') hover = null
  }
</script>

<div class="wrap">
  <div class="scroller">
    <svg width={svgW} height={svgH} role="grid" aria-label={ariaLabel} tabindex="0" onkeydown={key} onblur={() => (hover = null)}>
      {#each months as m (m.c)}
        <text class="axis" x={36 + m.c * (CELL + GAP)} y="10">{m.label}</text>
      {/each}
      {#each [0, 2, 4] as r (r)}
        <text class="axis" x="0" y={18 + r * (CELL + GAP) + CELL - 3}>{names[r]}</text>
      {/each}
      {#each days as d, i (d.date)}
        {@const p = pos(i)}
        {@const v = value(i)}
        <!-- keyboard access is provided by the grid (arrow keys + Enter) -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <rect
          x={36 + p.c * (CELL + GAP)}
          y={18 + p.r * (CELL + GAP)}
          width={CELL}
          height={CELL}
          rx="3"
          fill={ramp[level(v)]}
          class:sel={selected === d.date}
          class:hov={hover === i}
          role="gridcell"
          tabindex="-1"
          aria-label="{fmtDate(d.date, 'long')}: {format(v)}"
          onpointerenter={() => (hover = i)}
          onpointerleave={() => (hover = null)}
          onclick={() => onselect?.(d.date)}
        />
      {/each}
    </svg>
  </div>
  <div class="foot">
    {#if hover !== null}
      <span><b>{fmtDate(days[hover].date, 'long')}</b> · <span class="num">{format(value(hover))}</span></span>
    {:else}
      <span></span>
    {/if}
    <span class="scale" aria-hidden="true">
      {#each ramp as c (c)}<i style="background:{c}"></i>{/each}
    </span>
  </div>
</div>

<style>
  .wrap {
    width: 100%;
  }
  .scroller {
    overflow-x: auto;
    padding-bottom: 4px;
  }
  svg {
    display: block;
  }
  rect {
    stroke: transparent;
    stroke-width: 1.5;
    transition: stroke var(--dur) var(--ease);
  }
  rect.hov,
  rect.sel {
    stroke: var(--ink);
  }
  .axis {
    fill: var(--ink-3);
    font-size: 10.5px;
  }
  .foot {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 12px;
    color: var(--ink-2);
    min-height: 20px;
    margin-top: 6px;
  }
  .scale {
    display: inline-flex;
    gap: 3px;
  }
  .scale i {
    width: 11px;
    height: 11px;
    border-radius: 3px;
    display: inline-block;
  }
</style>
