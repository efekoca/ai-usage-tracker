<script lang="ts">
  import Icon from './Icon.svelte'
  import { t } from '../lib/i18n.svelte'
  import type { Snippet } from 'svelte'

  let { text = '', label = '', children }: { text?: string; label?: string; children?: Snippet } = $props()

  let open = $state(false)
  let right = $state(false)
  let root: HTMLSpanElement | undefined = $state()
  let button: HTMLButtonElement | undefined = $state()
  const id = `explain-${Math.random().toString(36).slice(2, 10)}`

  function toggle() {
    // a button on the right half of the window opens its text toward the left
    if (!open && root) right = root.getBoundingClientRect().left > window.innerWidth / 2
    open = !open
  }
  function outside(e: PointerEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false
  }
  function key(e: KeyboardEvent) {
    if (open && e.key === 'Escape') {
      open = false
      button?.focus()
    }
  }
</script>

<svelte:window onpointerdown={outside} onkeydown={key} />

<span class="explain" bind:this={root}>
  <button bind:this={button} type="button" class="q" aria-expanded={open} aria-controls={id} aria-label={label || t('explain.label')} title={label || t('explain.label')} onclick={toggle}>
    <Icon name="help" size={14} />
  </button>
  {#if open}
    <span class="pop" class:right {id} role="note">
      {#if children}{@render children()}{:else}{text}{/if}
    </span>
  {/if}
</span>

<style>
  .explain {
    position: relative;
    display: inline-flex;
    vertical-align: middle;
  }
  .q {
    display: inline-grid;
    place-items: center;
    width: 20px;
    height: 20px;
    margin: -2px 0;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--ink-3);
    cursor: pointer;
    transition:
      background var(--dur) var(--ease),
      color var(--dur) var(--ease);
  }
  .q:hover,
  .q[aria-expanded='true'] {
    background: var(--surface-hover);
    color: var(--ink);
  }
  .pop {
    position: absolute;
    top: calc(100% + 6px);
    left: -8px;
    z-index: 30;
    width: max-content;
    max-width: min(340px, 80vw);
    padding: 10px 12px;
    border: 0.5px solid var(--hairline-strong);
    border-radius: 11px;
    background: var(--surface);
    box-shadow: var(--shadow-pop);
    color: var(--ink-2);
    font-size: 12.5px;
    font-weight: 400;
    line-height: 1.5;
    letter-spacing: normal;
    text-align: left;
    text-transform: none;
    white-space: normal;
  }
  .pop.right {
    left: auto;
    right: -8px;
  }
  .pop :global(p) {
    margin: 0;
  }
  .pop :global(p + p) {
    margin-top: 6px;
  }
</style>
