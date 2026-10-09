<script lang="ts">
  import type { PlanAdvice, PlansFile } from '../lib/api'
  import { api } from '../lib/api'
  import { app, markTint } from '../lib/store.svelte'
  import { fmtDuration, fmtMoney, fmtPct, i18n, t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import BrandIcon from './BrandIcon.svelte'

  let { advice, plans, detected = false, compact = false }: { advice: PlanAdvice; plans: PlansFile | null; detected?: boolean; compact?: boolean } = $props()

  const a = $derived(advice)
  const list = $derived(plans?.providers[a.provider]?.plans ?? [])
  const name = (id: string | null) => list.find((p) => p.id === id)?.name ?? id ?? ''
  const source = $derived(list.find((p) => p.id === (a.suggested ?? a.plan))?.source ?? null)
  const pct = (v: number | null) => (v === null ? '—' : fmtPct(v))

  const tone = $derived(
    a.kind === 'upgrade' || a.kind === 'at_top' ? 'warn' : a.kind === 'downgrade' ? 'save' : a.kind === 'fits' ? 'ok' : 'info',
  )
  const icon = $derived(tone === 'warn' ? 'warning' : tone === 'ok' ? 'check' : tone === 'save' ? 'down' : 'info')
  const title = $derived(
    a.kind === 'upgrade'
      ? t(a.strong ? 'advice.kind.upgradeStrong' : 'advice.kind.upgrade', { plan: name(a.suggested) })
      : a.kind === 'downgrade'
        ? t('advice.kind.downgrade', { plan: name(a.suggested) })
        : t(`advice.kind.${a.kind}`),
  )
  const price = $derived(
    a.monthly_delta_usd === null
      ? ''
      : a.monthly_delta_usd > 0
        ? t('advice.price.more', { usd: fmtMoney(a.monthly_delta_usd) })
        : t('advice.price.less', { usd: fmtMoney(-a.monthly_delta_usd) }),
  )
  const why = $derived.by(() => {
    const out: string[] = []
    const from = name(a.plan)
    const to = name(a.suggested)
    switch (a.kind) {
      case 'upgrade':
        if (a.session_ratio) out.push(t('advice.why.ratio', { to, from, x: fmtNum(a.session_ratio) }))
        if (a.suggested_has_no_five_hour) out.push(t('advice.why.noFive', { to }))
        if (a.weekly.full > 0 && !a.suggested_has_no_five_hour) out.push(t('advice.why.weeklyUnknown'))
        break
      case 'downgrade':
        out.push(t('advice.why.down', { peak: pct(a.five_hour.peak_complete), to, proj: pct(a.projected_five_hour), x: fmtNum(a.session_ratio ?? 1) }))
        out.push(t('advice.why.downWeek', { peak: pct(a.weekly.peak_complete), proj: pct(a.projected_weekly_if_same_ratio) }))
        break
      case 'fits': {
        const lower = list.filter((p) => p.ladder !== undefined && p.ladder < (list.find((x) => x.id === a.plan)?.ladder ?? -1)).sort((x, y) => (y.ladder ?? 0) - (x.ladder ?? 0))[0]
        if (!lower) break
        if (a.session_ratio === null) out.push(t('advice.why.noRatio'))
        else if (a.projected_five_hour !== null && a.projected_five_hour > 80) out.push(t('advice.why.fitsProj', { to: lower.name, proj: pct(a.projected_five_hour) }))
        else out.push(t('advice.why.fitsNeedData'))
        break
      }
      case 'insufficient':
        out.push(t('advice.why.insufficient', { n: a.observed_days }))
        break
      case 'no_plan':
        out.push(t('advice.why.noPlan'))
        break
      case 'not_applicable':
        out.push(t('advice.why.notApplicable'))
        break
    }
    return out
  })
  function fmtNum(x: number) {
    return new Intl.NumberFormat(i18n.lang === 'tr' ? 'tr-TR' : 'en-US', { maximumFractionDigits: 2 }).format(x)
  }
  const facts = $derived(a.kind !== 'no_plan' && a.kind !== 'not_applicable' && (a.five_hour.windows > 0 || a.weekly.windows > 0))
</script>

<article class="advice {tone}" class:compact>
  <div class="head">
    <span class="mark" aria-hidden="true"><Icon name={icon} size={16} /></span>
    <div class="titles">
      <div class="who">
        <BrandIcon provider={a.provider} size={13} color={markTint(a.provider)} />{t(`provider.${a.provider}`)}{#if a.plan}{` · ${name(a.plan)}`}{/if}
        {#if detected}<span class="pill">{t('advice.detected')}</span>{/if}
      </div>
      <h3>{title}</h3>
      {#if price}<div class="price">{price}</div>{/if}
    </div>
  </div>
  {#if facts}
    <ul class="facts">
      {#if a.five_hour.windows > 0}
        <li>
          {t('advice.fact.five', { n: a.five_hour.windows, full: a.five_hour.full, peak: pct(a.five_hour.peak_seen) })}{#if a.five_hour.full_minutes > 0}{t('advice.fact.fiveFull', { d: fmtDuration(a.five_hour.full_minutes * 60000) })}{/if}
        </li>
      {/if}
      {#if a.weekly.windows > 0}
        <li>{t('advice.fact.week', { n: a.weekly.windows, full: a.weekly.full, peak: pct(a.weekly.peak_seen) })}</li>
      {/if}
      <li class="subtle">{t('advice.fact.days', { n: a.observed_days, of: a.days })}</li>
    </ul>
  {/if}
  {#each why as w (w)}<p class="why">{w}</p>{/each}
  {#if !compact}
    <div class="foot">
      {#if source}<button class="btn ghost small" onclick={() => api.openUrl(source)}><Icon name="external" size={13} />{t('advice.source')}</button>{/if}
      {#if a.kind === 'no_plan' && app.view !== 'limits'}<button class="btn ghost small" onclick={() => (app.view = 'limits')}>{t('nav.limits')}</button>{/if}
    </div>
  {:else if app.view !== 'limits'}
    <div class="foot"><button class="btn ghost small" onclick={() => { app.limitsTab = 'history'; app.view = 'limits' }}><Icon name="chevron" size={13} />{t('advice.open')}</button></div>
  {/if}
</article>

<style>
  .advice {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px 18px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 0.5px solid var(--hairline);
    box-shadow: var(--shadow);
    min-width: 0;
  }
  .head {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 9px;
    flex: none;
    background: var(--surface-hover);
    color: var(--ink-2);
  }
  .warn .mark {
    color: var(--bad-ink);
  }
  .ok .mark,
  .save .mark {
    color: var(--good-ink);
  }
  .titles {
    min-width: 0;
  }
  .who {
    font-size: 12px;
    color: var(--ink-2);
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  h3 {
    margin: 2px 0 0;
    font-size: 15px;
    font-weight: 600;
    line-height: 1.35;
  }
  .price {
    font-size: 13px;
    color: var(--ink-2);
    margin-top: 2px;
  }
  .pill {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 6px;
    background: var(--surface-hover);
  }
  .facts {
    margin: 0;
    padding: 0 0 0 42px;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  .why {
    margin: 0;
    padding-left: 42px;
    font-size: 13px;
    color: var(--ink-2);
    line-height: 1.45;
  }
  .foot {
    display: flex;
    gap: 8px;
    padding-left: 34px;
    flex-wrap: wrap;
  }
  .compact {
    padding: 14px 16px;
  }
</style>
