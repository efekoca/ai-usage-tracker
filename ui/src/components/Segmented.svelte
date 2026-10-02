<script lang="ts" generics="T extends string">
  // Apple-style segmented control: a radiogroup with roving focus and arrow-key navigation.
  let {
    options,
    value = $bindable(),
    label,
    onchange,
  }: { options: { value: T; label: string }[]; value: T; label: string; onchange?: (v: T) => void } = $props()

  let buttons: HTMLButtonElement[] = $state([])
  let root: HTMLDivElement | undefined = $state()
  const index = $derived(Math.max(0, options.findIndex((o) => o.value === value)))

  // segments can differ in width (labels vary by language), so the thumb follows the real button
  let thumb = $state({ left: 2, width: 0 })
  function measure() {
    const b = buttons[index]
    if (b) thumb = { left: b.offsetLeft, width: b.offsetWidth }
  }
  $effect(() => {
    void index
    void options.length
    measure()
    if (!root) return
    const ro = new ResizeObserver(measure)
    ro.observe(root)
    return () => ro.disconnect()
  })

  function select(i: number, focus = false) {
    const o = options[(i + options.length) % options.length]
    value = o.value
    onchange?.(o.value)
    if (focus) buttons[(i + options.length) % options.length]?.focus()
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
      e.preventDefault()
      select(index + 1, true)
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
      e.preventDefault()
      select(index - 1, true)
    } else if (e.key === 'Home') {
      e.preventDefault()
      select(0, true)
    } else if (e.key === 'End') {
      e.preventDefault()
      select(options.length - 1, true)
    }
  }
</script>

<div class="seg" role="radiogroup" aria-label={label} tabindex="-1" onkeydown={key} bind:this={root} style="--n:{options.length}">
  <span class="thumb" aria-hidden="true" style="width:{thumb.width}px;transform:translateX({thumb.left - 2}px)"></span>
  {#each options as o, i (o.value)}
    <button
      bind:this={buttons[i]}
      type="button"
      role="radio"
      aria-checked={o.value === value}
      tabindex={o.value === value ? 0 : -1}
      class:on={o.value === value}
      onclick={() => select(i)}>{o.label}</button
    >
  {/each}
</div>

<style>
  .seg {
    position: relative;
    display: inline-grid;
    grid-template-columns: repeat(var(--n), 1fr);
    padding: 2px;
    border-radius: 9px;
    background: var(--surface-hover);
    border: 0.5px solid var(--hairline);
    min-width: max-content;
  }
  .thumb {
    position: absolute;
    top: 2px;
    bottom: 2px;
    left: 2px;
    border-radius: 7px;
    background: var(--surface);
    box-shadow: 0 0.5px 1px rgba(0, 0, 0, 0.12), 0 2px 6px rgba(0, 0, 0, 0.06);
    transition: transform var(--dur) var(--ease);
  }
  button {
    position: relative;
    border: 0;
    background: transparent;
    height: 26px;
    padding: 0 12px;
    border-radius: 7px;
    font-size: 13px;
    font-weight: 500;
    color: var(--ink-2);
    white-space: nowrap;
  }
  button.on {
    color: var(--ink);
  }
  button:not(.on):hover {
    color: var(--ink);
  }
</style>
