<script lang="ts">
  import { app, saveSettings, toolColor } from '../lib/store.svelte'
  import type { Accuracy } from '../lib/api'
  import { fmtCompact, fmtDate, fmtHour, fmtMoney, fmtPct, limitName, t, toolLabel, weekdayNames } from '../lib/i18n.svelte'
  import StatTile from '../components/StatTile.svelte'
  import Segmented from '../components/Segmented.svelte'
  import AreaChart from '../components/charts/AreaChart.svelte'
  import Composition from '../components/charts/Composition.svelte'
  import BarList from '../components/charts/BarList.svelte'
  import LimitMeter from '../components/LimitMeter.svelte'
  import AccuracyBadge from '../components/AccuracyBadge.svelte'
  import Icon from '../components/Icon.svelte'
  import PlanValue from '../components/PlanValue.svelte'
  import LimitNotices from '../components/LimitNotices.svelte'

  let metric: 'tokens' | 'cost' = $state(app.settings?.primary_metric ?? 'tokens')
  const r = $derived(app.report)
  const toolsPresent = $derived((r?.by_tool ?? []).map((g) => g.key).filter((k) => k in toolColor).sort())
  const series = $derived(toolsPresent.map((k) => ({ key: k, label: toolLabel(k), color: toolColor[k] })))
  const values = $derived((r?.daily ?? []).map((d) => (metric === 'tokens' ? d.by_tool : d.cost_by_tool)))
  const fmt = $derived(metric === 'tokens' ? fmtCompact : (v: number) => fmtMoney(v, { compact: true }))
  const headline = $derived(
    app.limits.filter((l) => l.window === 'five_hour' || l.window === 'seven_day' || l.source === 'user_threshold'),
  )
  // the by-tool card joins the column that leaves the smaller difference at the bottom
  let hComposition = $state(0)
  let hModels = $state(0)
  let hLimits = $state(0)
  let hTools = $state(0)
  const byToolLeft = $derived.by(() => {
    const left = hComposition + 16 + hModels
    const tools = hTools + 16
    return Math.abs(left + tools - hLimits) < Math.abs(hLimits + tools - left)
  })
  const hasClaude = $derived(toolsPresent.includes('claude_code'))
  // dismissal covers only the listed models; a newly unpriced one still warns
  const unpriced = $derived((app.report?.unpriced_models ?? []).filter((m) => !(app.settings?.dismissed_unpriced ?? []).includes(m)))
  function dismissUnpriced() {
    const now = [...unpriced]
    saveSettings((c) => ({ dismissed_unpriced: [...new Set([...c.dismissed_unpriced, ...now])] }))
  }
  // the badge names the least certain source in the period; the tooltip gives the mix
  const accuracy = $derived.by(() => {
    const by = r?.by_accuracy ?? {}
    const kinds = (['estimated', 'captured', 'exact'] as Accuracy[]).filter((k) => (by[k] ?? 0) > 0)
    const total = kinds.reduce((a, k) => a + by[k], 0)
    return {
      kind: kinds[0] ?? 'exact',
      detail: kinds.length > 1 ? kinds.map((k) => `${t(`acc.${k}`)} ${fmtPct((by[k] / total) * 100, 0)}`).join(' · ') : '',
    }
  })
</script>

{#if !r}
  <p class="muted">{t('common.loading')}</p>
{:else if r.totals.events === 0}
  <div class="empty card">
    <Icon name="overview" size={28} />
    <p>{t('common.empty')}</p>
  </div>
{:else}
  {#if unpriced.length}
    <div class="banner" role="status">
      <Icon name="info" size={16} />
      <span>{t('overview.unpriced', { n: unpriced.length, models: unpriced.join(', ') })}</span>
      <span class="spacer"></span>
      <button class="btn" onclick={() => (app.view = 'settings')}>{t('overview.unpricedAction')}</button>
      <button class="btn ghost dismiss" aria-label={t('overview.unpricedDismiss')} title={t('overview.unpricedDismiss')} onclick={dismissUnpriced}><Icon name="close" size={14} /></button>
    </div>
  {/if}

  <section class="tiles card">
    <StatTile hero label={t('overview.totalTokens')} value={fmtCompact(r.totals.total_tokens)} current={r.totals.total_tokens} previous={r.previous.total_tokens}>
      <AccuracyBadge kind={accuracy.kind} detail={accuracy.detail} />
    </StatTile>
    <StatTile label={t('overview.cost')} hint={t('metric.apiEq.help')} value={fmtMoney(r.totals.cost_usd)} current={r.totals.cost_usd} previous={r.previous.cost_usd} />
    <StatTile label={t('overview.avgDaily')} value={fmtCompact(r.avg_daily_tokens)}>
      <span class="subtle">{fmtMoney(r.avg_daily_cost)}</span>
    </StatTile>
    <StatTile label={t('overview.activeDays')} value="{r.active_days} / {r.days_in_range}">
      {#if r.peak_day}<span class="subtle">{t('overview.peakDay')}: {fmtDate(r.peak_day.date, 'short')}</span>{/if}
    </StatTile>
  </section>

  <section class="card">
    <div class="card-head">
      <h2>{t('overview.trend')}</h2>
      <span class="spacer"></span>
      <Segmented label={t('overview.trend')} options={[{ value: 'tokens', label: t('metric.tokens') }, { value: 'cost', label: t('metric.cost') }]} bind:value={metric} />
    </div>
    <AreaChart
      dates={r.daily.map((d) => d.date)}
      {values}
      {series}
      format={fmt}
      ariaLabel="{t('overview.trend')}: {metric === 'tokens' ? fmtCompact(r.totals.total_tokens) : fmtMoney(r.totals.cost_usd)}"
    />
  </section>

  <div class="plan"><PlanValue /></div>

  {#snippet byTool()}
    <section class="card" bind:offsetHeight={hTools}>
      <div class="card-head"><h2>{t('overview.byTool')}</h2></div>
      <BarList
        ariaLabel={t('overview.byTool')}
        items={r.by_tool.map((g) => ({ key: g.key, label: toolLabel(g.key), value: metric === 'tokens' ? g.totals.total_tokens : g.totals.cost_usd, color: toolColor[g.key] }))}
        format={fmt}
      />
      <div class="facts small muted">
        {#if r.peak_hour !== null}<span>{t('overview.peakHour')}: <b>{fmtHour(r.peak_hour)}</b></span>{/if}
        {#if r.peak_weekday !== null}<span>{weekdayNames()[r.peak_weekday]}</span>{/if}
      </div>
    </section>
  {/snippet}

  <div class="grid2">
    <div class="col">
      <section class="card" bind:offsetHeight={hComposition}>
        <div class="card-head"><h2>{t('overview.composition')}</h2></div>
        <Composition tokens={r.totals.tokens} />
      </section>
      <section class="card" bind:offsetHeight={hModels}>
        <div class="card-head"><h2>{t('overview.topModels')}</h2></div>
        <BarList
          ariaLabel={t('overview.topModels')}
          max={6}
          items={r.by_model.map((g) => ({ key: g.key, label: g.label, value: metric === 'tokens' ? g.totals.total_tokens : g.totals.cost_usd, sub: g.totals.unpriced_events ? '—' : undefined }))}
          format={fmt}
        />
      </section>
      {#if byToolLeft}{@render byTool()}{/if}
    </div>
    <div class="col">
      <section class="card" bind:offsetHeight={hLimits}>
        <div class="card-head">
          <h2>{t('overview.limitsNow')}</h2>
          <span class="spacer"></span>
          <button class="btn ghost" aria-label={t('overview.openLimits')} title={t('overview.openLimits')} onclick={() => (app.view = 'limits')}><Icon name="chevron" size={14} /></button>
        </div>
        <LimitNotices />
        {#if headline.length === 0}
          <p class="muted small">{t('limits.none')}</p>
        {:else}
          <div class="meters">
            {#each headline as l (l.provider + l.limit_id + l.window)}
              <LimitMeter
                compact
                title="{limitName(l.provider, l.limit_id)} · {t(`limits.window.${l.window}`)}"
                window={l.window}
                used={l.used_pct}
                state={l.state}
                accuracy={l.accuracy}
                resetsAt={l.resets_at}
                observedMs={l.observed_ms}
                forecast={l.forecast}
              />
            {/each}
          </div>
        {/if}
      </section>
      {#if !byToolLeft}{@render byTool()}{/if}
    </div>
  </div>

  {#if hasClaude}
    <p class="footnote subtle small"><Icon name="info" size={13} /> {t('overview.excludesAux')}</p>
  {/if}
{/if}

<style>
  section {
    margin-bottom: 16px;
  }
  .tiles {
    display: grid;
    grid-template-columns: 1.4fr 1fr 1fr 1fr;
    gap: 24px;
  }
  @container main (max-width: 940px) {
    .tiles {
      grid-template-columns: 1fr 1fr;
    }
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
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    align-items: start;
    margin-bottom: 16px;
  }
  .plan {
    margin-bottom: 16px;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
  }
  .col section {
    margin-bottom: 0;
  }
  @container main (max-width: 900px) {
    .grid2 {
      grid-template-columns: 1fr;
    }
  }
  .meters {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .banner {
    margin-bottom: 16px;
    align-items: center;
  }
  .dismiss {
    width: 28px;
    height: 28px;
    padding: 0;
    justify-content: center;
    margin-right: -6px;
  }
  .facts {
    display: flex;
    gap: 14px;
    margin-top: 10px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 60px;
    color: var(--ink-2);
  }
  .footnote {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    max-width: 760px;
  }
</style>
