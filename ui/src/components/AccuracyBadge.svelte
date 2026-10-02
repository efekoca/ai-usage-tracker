<script lang="ts">
  // "Exact / Estimated / Captured" — always icon + text, never color alone.
  import Icon from './Icon.svelte'
  import { t } from '../lib/i18n.svelte'
  import type { Accuracy } from '../lib/api'
  let { kind, compact = false }: { kind: Accuracy; compact?: boolean } = $props()
</script>

<span class="badge {kind}" title={t(`acc.${kind}.help`)}>
  <Icon name={kind} size={compact ? 13 : 14} />
  {#if !compact}<span>{t(`acc.${kind}`)}</span>{:else}<span class="sr-only">{t(`acc.${kind}`)}</span>{/if}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 7px 0 5px;
    border-radius: 6px;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--ink-2);
    background: var(--surface-hover);
    white-space: nowrap;
    vertical-align: middle;
  }
  .badge.exact :global(svg) {
    color: var(--good-ink);
  }
  .badge.estimated {
    background: transparent;
    border: 1px dashed var(--hairline-strong);
  }
</style>
