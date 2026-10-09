<script lang="ts">
  import { fmtHour, weekdayNames } from '../../lib/i18n.svelte'
  let { grid, format, ariaLabel }: { grid: number[][]; format: (v: number) => string; ariaLabel: string } = $props()

  let hover: [number, number] | null = $state(null)
  // one tab stop for the whole grid; arrow keys move it
  let cursor: [number, number] = $state([0, 0])
  let cells: HTMLElement[][] = $state([])
  const max = $derived(Math.max(1, ...grid.flat()))
  const ramp = ['var(--seq-0)', 'var(--seq-1)', 'var(--seq-2)', 'var(--seq-3)', 'var(--seq-4)', 'var(--seq-5)', 'var(--seq-6)']
  const color = (v: number) => (v <= 0 ? ramp[0] : ramp[Math.min(6, 1 + Math.floor((Math.sqrt(v / max)) * 5.999))])
  const names = $derived(weekdayNames())

  function move(e: KeyboardEvent, d: number, h: number) {
    const last = grid.length - 1
    const next: Record<string, [number, number]> = {
      ArrowLeft: [d, Math.max(0, h - 1)],
      ArrowRight: [d, Math.min(23, h + 1)],
      ArrowUp: [Math.max(0, d - 1), h],
      ArrowDown: [Math.min(last, d + 1), h],
      Home: [d, 0],
      End: [d, 23],
    }
    const to = next[e.key]
    if (!to) return
    e.preventDefault()
    cursor = to
    cells[to[0]]?.[to[1]]?.focus()
  }
</script>

<div class="hm" role="grid" aria-label={ariaLabel}>
  <div class="row" role="row">
    <div role="columnheader"></div>
    {#each Array(24) as _, h (h)}
      <div class="hr" role="columnheader" aria-label={fmtHour(h)}>{h % 3 === 0 ? String(h).padStart(2, '0') : ''}</div>
    {/each}
  </div>
  {#each grid as row, d (d)}
    <div class="row" role="row">
      <div class="dn" role="rowheader">{names[d]}</div>
      {#each row as v, h (h)}
        <div
          class="c"
          role="gridcell"
          tabindex={cursor[0] === d && cursor[1] === h ? 0 : -1}
          aria-label="{names[d]} {fmtHour(h)}: {format(v)}"
          style="background:{color(v)}"
          class:on={hover?.[0] === d && hover?.[1] === h}
          bind:this={() => cells[d]?.[h], (el) => ((cells[d] ??= [])[h] = el)}
          onpointerenter={() => (hover = [d, h])}
          onpointerleave={() => (hover = null)}
          onfocus={() => ((cursor = [d, h]), (hover = [d, h]))}
          onblur={() => (hover = null)}
          onkeydown={(e) => move(e, d, h)}
        ></div>
      {/each}
    </div>
  {/each}
</div>
<div class="foot" aria-live="polite">
  {#if hover}
    <span><b>{names[hover[0]]} {fmtHour(hover[1])}</b> · <span class="num">{format(grid[hover[0]]?.[hover[1]] ?? 0)}</span></span>
  {:else}
    <span></span>
  {/if}
</div>

<style>
  .hm {
    display: grid;
    grid-template-columns: 36px repeat(24, minmax(10px, 1fr));
    gap: 3px;
    align-items: center;
  }
  .row {
    display: contents;
  }
  .hr,
  .dn {
    font-size: 10.5px;
    color: var(--ink-3);
  }
  .c {
    aspect-ratio: 1;
    border-radius: 3px;
    outline: 1.5px solid transparent;
    max-height: 22px;
  }
  .c.on,
  .c:focus-visible {
    outline-color: var(--ink);
  }
  .foot {
    font-size: 12px;
    color: var(--ink-2);
    min-height: 20px;
    margin-top: 6px;
  }
</style>
