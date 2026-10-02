<script lang="ts">
  // Ranked horizontal bars (single hue): label · bar · value. Long tails fold into "Other".
  import { t } from '../../lib/i18n.svelte'
  type Item = { key: string; label: string; value: number; sub?: string; color?: string; muted?: boolean }
  let {
    items,
    format,
    max = 8,
    onpick,
    ariaLabel,
  }: { items: Item[]; format: (v: number) => string; max?: number; onpick?: (key: string) => void; ariaLabel: string } = $props()

  const shown = $derived.by(() => {
    const sorted = [...items].sort((a, b) => b.value - a.value)
    if (sorted.length <= max) return sorted
    const head = sorted.slice(0, max - 1)
    const rest = sorted.slice(max - 1)
    return [...head, { key: '__other', label: `${t('common.other')} (${rest.length})`, value: rest.reduce((a, r) => a + r.value, 0), muted: true }]
  })
  const top = $derived(Math.max(1e-9, ...shown.map((s) => s.value)))
</script>

<ul class="bars" aria-label={ariaLabel}>
  {#each shown as it (it.key)}
    <li>
      <svelte:element
        this={onpick && it.key !== '__other' ? 'button' : 'div'}
        class="row"
        type={onpick && it.key !== '__other' ? 'button' : undefined}
        onclick={onpick && it.key !== '__other' ? () => onpick(it.key) : undefined}
        role={onpick && it.key !== '__other' ? undefined : 'presentation'}
      >
        <span class="lbl" class:muted={it.muted}>
          {#if it.color}<i class="swatch" style="background:{it.color}"></i>{/if}
          <span class="txt">{it.label}</span>
          {#if it.sub}<span class="sub">{it.sub}</span>{/if}
        </span>
        <span class="val num">{format(it.value)}</span>
        <span class="track" aria-hidden="true"><span class="fill" style="width:{Math.max(0.5, (it.value / top) * 100)}%"></span></span>
      </svelte:element>
    </li>
  {/each}
</ul>

<style>
  .bars {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .row {
    width: 100%;
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 4px 12px;
    padding: 6px 8px;
    margin: 0 -8px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    text-align: left;
    width: calc(100% + 16px);
  }
  button.row:hover {
    background: var(--surface-hover);
  }
  .lbl {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-size: 13px;
  }
  .lbl.muted {
    color: var(--ink-2);
  }
  .txt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    color: var(--ink-3);
    font-size: 12px;
    white-space: nowrap;
  }
  .val {
    font-size: 13px;
    color: var(--ink-2);
  }
  .track {
    grid-column: 1 / -1;
    height: 6px;
    border-radius: 3px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: 3px;
    background: var(--accent);
    transition: width 500ms var(--ease);
  }
  .muted + .val + .track .fill {
    background: var(--ink-3);
  }
</style>
