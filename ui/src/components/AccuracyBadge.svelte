<script lang="ts">
  import Icon from './Icon.svelte'
  import { t } from '../lib/i18n.svelte'
  import type { Accuracy } from '../lib/api'
  let { kind, compact = false }: { kind: Accuracy; compact?: boolean } = $props()
</script>

<span class="badge {kind}" class:compact title={t(`acc.${kind}.help`)}>
  <Icon name={kind} size={compact ? 12 : 13} />
  <span>{t(`acc.${kind}`)}</span>
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 8px 0 6px;
    border-radius: 999px;
    font-size: 11.5px;
    font-weight: 500;
    letter-spacing: 0.01em;
    color: var(--ink-2);
    background: var(--surface-hover);
    white-space: nowrap;
    vertical-align: middle;
    flex: none;
  }
  .badge.compact {
    height: 18px;
    font-size: 11px;
    padding: 0 7px 0 5px;
  }
  .badge :global(svg) {
    color: var(--ink-3);
  }
  .badge.exact :global(svg) {
    color: var(--good-ink);
  }
</style>
