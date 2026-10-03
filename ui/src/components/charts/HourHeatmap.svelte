<script lang="ts">
  import { fmtHour, weekdayNames } from '../../lib/i18n.svelte'
  let { grid, format, ariaLabel }: { grid: number[][]; format: (v: number) => string; ariaLabel: string } = $props()

  let hover: [number, number] | null = $state(null)
  const max = $derived(Math.max(1, ...grid.flat()))
  const ramp = ['var(--seq-0)', 'var(--seq-1)', 'var(--seq-2)', 'var(--seq-3)', 'var(--seq-4)', 'var(--seq-5)', 'var(--seq-6)']
  const color = (v: number) => (v <= 0 ? ramp[0] : ramp[Math.min(6, 1 + Math.floor((Math.sqrt(v / max)) * 5.999))])
  const names = $derived(weekdayNames())
</script>

<div class="hm" role="grid" aria-label={ariaLabel}>
  <div></div>
  {#each Array(24) as _, h (h)}
    <div class="hr" aria-hidden="true">{h % 3 === 0 ? String(h).padStart(2, '0') : ''}</div>
  {/each}
  {#each grid as row, d (d)}
    <div class="dn">{names[d]}</div>
    {#each row as v, h (h)}
      <div
        class="c"
        role="gridcell"
        tabindex="-1"
        aria-label="{names[d]} {fmtHour(h)}: {format(v)}"
        style="background:{color(v)}"
        class:on={hover?.[0] === d && hover?.[1] === h}
        onpointerenter={() => (hover = [d, h])}
        onpointerleave={() => (hover = null)}
      ></div>
    {/each}
  {/each}
</div>
<div class="foot">
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
  .c.on {
    outline-color: var(--ink);
  }
  .foot {
    font-size: 12px;
    color: var(--ink-2);
    min-height: 20px;
    margin-top: 6px;
  }
</style>
