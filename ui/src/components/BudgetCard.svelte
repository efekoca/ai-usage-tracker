<script lang="ts">
  import { app, latest } from '../lib/store.svelte'
  import { api, type MonthSpend } from '../lib/api'
  import { fmtDate, fmtMoney, fmtPct, t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'

  let spend = $state<MonthSpend | null>(null)
  let failed = $state('')

  $effect(() => {
    void app.tick
    return latest(
      () => api.monthSpend(),
      (v) => {
        spend = v
        failed = ''
      },
      (e) => (failed = String(e)),
    )
  })

  const budget = $derived(app.settings?.monthly_budget_usd ?? 0)
  const used = $derived(spend ? (spend.spent_usd / budget) * 100 : 0)
  const pace = $derived(spend?.projected_usd != null ? (spend.projected_usd / budget) * 100 : null)
  const level = $derived(used >= 100 ? 'high' : used >= 80 || (pace ?? 0) > 100 ? 'warn' : 'ok')
  const daysLeft = $derived(spend ? Math.max(0, Math.ceil((spend.ends_ms - Date.now()) / 86_400_000)) : 0)
</script>

<section class="card">
  <div class="card-head">
    <h2>{t('budget.title')}</h2>
    <span class="spacer"></span>
    {#if spend}<span class="subtle small">{fmtDate(spend.month, 'monthLong')} · {t('budget.left', { n: daysLeft })}</span>{/if}
    <button class="btn ghost" aria-label={t('budget.edit')} title={t('budget.edit')} onclick={() => (app.view = 'settings')}><Icon name="gear" size={14} /></button>
  </div>
  {#if failed && !spend}
    <p class="muted small">{t('common.error', { e: failed })}</p>
  {:else if !spend}
    <p class="muted small">{t('common.loading')}</p>
  {:else}
    <div class="figures">
      <span class="spent num">{fmtMoney(spend.spent_usd)}</span>
      <span class="subtle">/ {fmtMoney(budget)}</span>
      <span class="spacer"></span>
      <span class="pct num {level}">{fmtPct(used)}</span>
    </div>
    <div class="track" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.min(100, Math.round(used))} aria-valuetext={fmtPct(used)} aria-label={t('budget.title')}>
      {#if pace !== null && pace > used}<span class="pace {level}" style="width:{Math.min(100, pace)}%"></span>{/if}
      <span class="fill {level}" style="width:{Math.min(100, used)}%"></span>
    </div>
    <p class="foot small">
      {#if used >= 100}
        <span class="high">{t('budget.over', { v: fmtMoney(spend.spent_usd - budget) })}</span>
      {:else if spend.projected_usd !== null}
        <span class:warn={(pace ?? 0) > 100}>{t('budget.pace', { v: fmtMoney(spend.projected_usd) })}</span>
        <AccuracyBadge kind="estimated" compact />
      {:else}
        <span class="subtle">{t('budget.early')}</span>
      {/if}
    </p>
    <p class="subtle small note">{t('budget.note')}</p>
  {/if}
</section>

<style>
  section {
    margin-bottom: 16px;
  }
  .card-head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    min-height: 30px;
  }
  .spacer {
    flex: 1;
  }
  .figures {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin-bottom: 8px;
  }
  .spent {
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.02em;
  }
  .pct {
    font-size: 15px;
    font-weight: 650;
  }
  .pct.warn,
  .warn {
    color: var(--serious);
  }
  .pct.high,
  .high {
    color: var(--critical);
  }
  .track {
    position: relative;
    height: 8px;
    border-radius: 4px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .fill,
  .pace {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 4px;
    background: var(--accent);
    transition: width 600ms var(--ease);
  }
  /* where the month ends at this pace, behind what is spent */
  .pace {
    opacity: 0.3;
  }
  .fill.warn,
  .pace.warn {
    background: var(--serious);
  }
  .fill.high,
  .pace.high {
    background: var(--critical);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 10px 0 0;
  }
  .note {
    margin: 4px 0 0;
  }
</style>
