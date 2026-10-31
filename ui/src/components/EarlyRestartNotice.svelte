<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, type EarlyRestart } from '../lib/api'
  import { fmtDate, t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'

  let early = $state<EarlyRestart | null>(null)
  let open = $state(false)

  $effect(() => {
    void app.tick
    api.earlyRestart().then((e) => (early = e)).catch(() => {})
  })

  const days = $derived(early ? Math.round((early.resets_at_ms - early.at_ms) / 86_400_000) : 0)
  const capture = $derived(app.settings?.capture)
  // an idle Claude is read no more often than every 15 minutes
  const wait = $derived(Math.max(15, capture?.claude_poll_minutes ?? 15))
</script>

{#if early}
  <div class="banner col" role="status">
    <div class="line">
      <Icon name="info" size={16} />
      <span>{t('limits.early', { date: fmtDate(early.at_ms, 'short'), n: days })}</span>
      <span class="spacer"></span>
      <button class="btn small" aria-expanded={open} onclick={() => (open = !open)}>{t('limits.early.how')}</button>
    </div>
    {#if open}
      <ol class="small">
        <li>{t('limits.early.step1')}</li>
        <li>{capture?.claude_poll ? t('limits.early.step2', { n: wait }) : t('limits.early.step2Off')}</li>
        <li>{t('limits.early.step3')}</li>
      </ol>
      <p class="subtle small">{t('limits.early.after')}</p>
    {/if}
  </div>
{/if}

<style>
  .banner {
    align-items: center;
    margin-bottom: 16px;
  }
  .banner.col {
    flex-direction: column;
    align-items: stretch;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  ol {
    margin: 4px 0 0;
    padding-left: 44px;
    display: grid;
    gap: 4px;
  }
  p {
    margin: 0;
    padding-left: 24px;
  }
</style>
