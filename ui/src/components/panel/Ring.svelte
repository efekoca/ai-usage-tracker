<script lang="ts">
  import type { Snippet } from 'svelte'

  // `value` null draws only the track: there is no reading to show
  let { value, color, size = 48, stroke = 5, children }: { value: number | null; color: string; size?: number; stroke?: number; children?: Snippet } = $props()

  const r = $derived((size - stroke) / 2)
  const c = $derived(2 * Math.PI * r)
  const filled = $derived(value === null ? 0 : (Math.min(100, Math.max(0, value)) / 100) * c)
</script>

<span class="ring" style:width="{size}px" style:height="{size}px">
  <svg width={size} height={size} viewBox="0 0 {size} {size}" aria-hidden="true">
    <circle class="track" class:empty={value === null} cx={size / 2} cy={size / 2} {r} stroke-width={stroke} />
    {#if value !== null && value > 0}
      <circle class="arc" cx={size / 2} cy={size / 2} {r} stroke-width={stroke} stroke={color} stroke-dasharray="{filled} {c}" transform="rotate(-90 {size / 2} {size / 2})" />
    {/if}
  </svg>
  {#if children}<span class="center">{@render children()}</span>{/if}
</span>

<style>
  .ring {
    position: relative;
    display: inline-grid;
    place-items: center;
    flex: none;
  }
  svg {
    position: absolute;
    inset: 0;
  }
  circle {
    fill: none;
  }
  .track {
    stroke: var(--cc-track);
  }
  .track.empty {
    stroke-dasharray: 2 4;
  }
  .arc {
    stroke-linecap: round;
    transition: stroke-dasharray 600ms var(--ease);
  }
  .center {
    position: relative;
    display: grid;
    place-items: center;
    text-align: center;
    line-height: 1;
  }
  @media (prefers-reduced-motion: reduce) {
    .arc {
      transition: none;
    }
  }
</style>
