<script lang="ts">
  import { app, navigate, setFilter, setPeriod } from '../lib/store.svelte'
  import { t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'

  let { icon = 'overview', text = '' }: { icon?: string; text?: string } = $props()

  const filtered = $derived(Object.values(app.filter).some((v) => Array.isArray(v) && v.length > 0))
</script>

<div class="empty card">
  <Icon name={icon} size={28} />
  <p>{text || t('common.empty')}</p>
  <div class="actions">
    {#if filtered}<button class="btn" onclick={() => setFilter({})}>{t('empty.clearFilters')}</button>{/if}
    {#if app.period.kind !== 'all'}<button class="btn" onclick={() => setPeriod({ kind: 'all' })}>{t('empty.allTime')}</button>{/if}
    <button class="btn ghost" onclick={() => navigate('sources', { from: true })}>{t('empty.sources')}</button>
  </div>
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 56px 24px;
    color: var(--ink-2);
    text-align: center;
  }
  .empty p {
    margin: 0;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin-top: 6px;
  }
</style>
