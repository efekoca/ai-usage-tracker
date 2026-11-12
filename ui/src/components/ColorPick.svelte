<script lang="ts" module>
  /** A color and its name, which screen readers say instead of the hex code. */
  export type Swatch = [color: string, name: 'orange' | 'green' | 'blue' | 'purple' | 'pink' | 'gray']
</script>

<script lang="ts">
  import { t } from '../lib/i18n.svelte'

  // '' is the theme's accent color
  let {
    value,
    label,
    defaultLabel,
    customLabel,
    swatches,
    onchange,
  }: { value: string; label: string; defaultLabel: string; customLabel: string; swatches: Swatch[]; onchange: (v: string) => void } = $props()
</script>

<div class="row pick" role="group" aria-label={label}>
  {#each [['', ''] as const, ...swatches] as [c, name] (c)}
    {@const said = name ? t(`color.${name}`) : defaultLabel}
    <button class="swatchbtn" class:sel={value.toLowerCase() === c} style="--c:{c || 'var(--accent)'}" aria-label={said} aria-pressed={value.toLowerCase() === c} title={said} onclick={() => onchange(c)}>
      {#if !c}<span aria-hidden="true">A</span>{/if}
    </button>
  {/each}
  <input type="color" class="picker" value={value || '#2f6feb'} aria-label="{label} · {customLabel}" onchange={(e) => onchange(e.currentTarget.value)} />
</div>

<style>
  .pick {
    gap: 8px;
    flex: none;
  }
  .swatchbtn {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 2px solid var(--surface);
    outline: 1px solid var(--hairline-strong);
    background: var(--c);
    display: grid;
    place-items: center;
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    padding: 0;
  }
  .swatchbtn.sel {
    outline: 2px solid var(--ink);
  }
  .picker {
    width: 30px;
    height: 26px;
    border: 0;
    background: transparent;
    padding: 0;
  }
</style>
