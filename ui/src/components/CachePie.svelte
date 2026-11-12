<script lang="ts">
  import type { CacheTone } from '../lib/caches'

  // the slice left runs from the hand back to twelve, as on a kitchen timer
  let { frac, tone = 'ok', busy = false, size = 20 }: { frac: number; tone?: CacheTone; busy?: boolean; size?: number } = $props()

  const c = $derived(size / 2)
  const ring = $derived(Math.max(1.5, size * 0.07))
  const r = $derived(c - ring / 2)
  const rp = $derived(c - ring - Math.max(1.5, size * 0.075))
  const f = $derived(Math.max(0, Math.min(1, frac)))
  const pt = (a: number) => `${(c + rp * Math.sin((a * Math.PI) / 180)).toFixed(3)} ${(c - rp * Math.cos((a * Math.PI) / 180)).toFixed(3)}`
  const d = $derived.by(() => {
    const from = busy ? -90 : 360 * (1 - f)
    const sweep = busy ? 90 : 360 - from
    return sweep < 0.5 ? '' : `M${c} ${c}L${pt(from)}A${rp} ${rp} 0 ${sweep > 180 ? 1 : 0} 1 ${pt(from + sweep)}Z`
  })
</script>

<svg class="pie {tone}" class:busy width={size} height={size} viewBox="0 0 {size} {size}" aria-hidden="true">
  <circle class="rim" cx={c} cy={c} {r} stroke-width={ring} />
  {#if !busy && f > 0.999}
    <circle class="slice" cx={c} cy={c} r={rp} />
  {:else if d}
    <path class="slice" {d} />
  {/if}
</svg>

<style>
  .pie {
    flex: none;
    display: block;
    color: var(--accent);
  }
  .pie.soon {
    color: var(--warning);
  }
  .pie.urgent {
    color: var(--critical);
  }
  .rim {
    fill: none;
    stroke: currentColor;
  }
  .slice {
    fill: currentColor;
  }
  .busy .slice {
    transform-box: view-box;
    transform-origin: 50% 50%;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .busy .slice {
      animation: none;
    }
  }
</style>
