<script lang="ts">
  // '' is the theme's accent color
  let {
    value,
    label,
    defaultLabel,
    customLabel,
    swatches,
    onchange,
  }: { value: string; label: string; defaultLabel: string; customLabel: string; swatches: string[]; onchange: (v: string) => void } = $props()
</script>

<div class="row pick" role="group" aria-label={label}>
  {#each ['', ...swatches] as c (c)}
    <button class="swatchbtn" class:sel={value.toLowerCase() === c} style="--c:{c || 'var(--accent)'}" aria-label={c || defaultLabel} aria-pressed={value.toLowerCase() === c} title={c || defaultLabel} onclick={() => onchange(c)}>
      {#if !c}<span aria-hidden="true">A</span>{/if}
    </button>
  {/each}
  <input type="color" class="picker" value={value || '#2a78d6'} aria-label="{label} · {customLabel}" onchange={(e) => onchange(e.currentTarget.value)} />
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
