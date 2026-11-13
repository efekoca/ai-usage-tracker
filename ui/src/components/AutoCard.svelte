<script lang="ts">
  import type { Snippet } from 'svelte'
  import Toggle from './Toggle.svelte'

  // a card for something the app does by itself: what it is, whether it is on, and its details below
  let {
    title,
    help,
    tag = '',
    on,
    disabled = false,
    onchange,
    icon,
    children,
  }: { title: string; help: string; tag?: string; on: boolean; disabled?: boolean; onchange: (v: boolean) => void; icon: Snippet; children?: Snippet } = $props()
</script>

<section class="card auto" class:off={!on}>
  <div class="head">
    <span class="tile" aria-hidden="true">{@render icon()}</span>
    <div class="what">
      <div class="line">
        <h2>{title}</h2>
        {#if tag}<span class="tag">{tag}</span>{/if}
      </div>
      <p class="subtle small">{help}</p>
    </div>
    <Toggle checked={on} {disabled} label={title} {onchange} />
  </div>
  {@render children?.()}
</section>

<style>
  .auto {
    --tint: color-mix(in srgb, var(--accent) 15%, transparent);
    --tint-edge: color-mix(in srgb, var(--accent) 38%, transparent);
    --tint-ink: color-mix(in srgb, var(--accent) 62%, var(--ink));
    display: flex;
    flex-direction: column;
    gap: 16px;
    margin-bottom: 16px;
    padding: 20px 22px 18px;
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: 14px;
  }
  .tile {
    flex: none;
    width: 38px;
    height: 38px;
    border-radius: 11px;
    display: grid;
    place-items: center;
    background: var(--tint);
    color: var(--tint-ink);
  }
  .off .tile {
    background: var(--surface-2);
    color: var(--ink-3);
  }
  .what {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 4px;
  }
  .line {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 10px;
  }
  .what p {
    margin: 0;
    max-width: 640px;
  }
  .tag {
    padding: 2px 9px;
    border-radius: 999px;
    background: var(--tint);
    color: var(--tint-ink);
    font-size: 11.5px;
    font-weight: 600;
    white-space: nowrap;
  }
  /* the parts the cards share */
  .auto :global(.inset) {
    border-radius: 12px;
    background: var(--stage);
    border: 0.5px solid var(--hairline);
  }
  .auto :global(.inset > .row) {
    display: grid;
    align-items: center;
    gap: 14px;
    padding: 12px 16px;
    border-bottom: 0.5px solid var(--hairline);
  }
  .auto :global(.inset > .row:last-child) {
    border-bottom: 0;
  }
  .auto :global(.cap) {
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: var(--ink-3);
  }
  .auto :global(.note) {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    margin: 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--ink-3);
  }
  .auto :global(.note svg) {
    flex: none;
    margin-top: 1px;
  }
</style>
