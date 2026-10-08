<script lang="ts">
  // Rendered in a hidden window and saved as a PDF by the app.
  import { onMount, tick } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { applyAppearance, toolColor } from './lib/store.svelte'
  import { api, providerOfSource, providersOf, type ContextStats, type LimitView, type PlansFile, type Provider, type Report, type Sessions, type Settings } from './lib/api'
  import { fmtCompact, fmtDate, fmtDateTime, fmtDec, fmtInt, fmtLimit, fmtMoney, fmtPct, fmtSpan, i18n, t, toolLabel, windowLabel } from './lib/i18n.svelte'
  import Columns from './components/charts/Columns.svelte'

  const qs = new URLSearchParams(location.search)
  const from = qs.get('from') ?? ''
  const to = qs.get('to') ?? ''

  let s = $state<Settings | null>(null)
  let r = $state<Report | null>(null)
  let sessions = $state<Sessions | null>(null)
  let ctx = $state<ContextStats | null>(null)
  let limits = $state<LimitView[]>([])
  let plans = $state<PlansFile | null>(null)
  let failed = $state('')

  onMount(async () => {
    try {
      s = await api.getSettings()
      applyAppearance(s, null)
      // paper is always light
      document.documentElement.dataset.theme = 'light'
      const period = { kind: 'custom' as const, from, to }
      ;[r, sessions, ctx, limits, plans] = await Promise.all([api.report(period), api.sessions(period), api.contextStats(period), api.limits(), api.plans()])
    } catch (e) {
      failed = String(e)
    }
    await tick()
    await document.fonts.ready
    // let the charts measure their width and draw before printing; a hidden macOS window gets no frames
    await Promise.race([
      new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(res))),
      new Promise((res) => setTimeout(res, 500)),
    ])
    // a page without its data must not be saved as a report
    invoke('report_ready', { ok: !failed }).catch(() => {})
  })

  const mode = $derived(s?.limit_display ?? 'used')
  const hideAll = $derived(!!s?.hide_project_names)
  const prompt = $derived(r ? r.totals.tokens.input + r.totals.tokens.cache_read + r.totals.tokens.cache_write : 0)
  const change = (cur: number, prev: number) => (prev > 0 ? ((cur - prev) / prev) * 100 : null)
  const series = $derived(
    (r?.by_tool ?? []).map((g) => ({ key: g.key, label: toolLabel(g.key), color: toolColor[g.key] ?? 'var(--s5)' })),
  )
  const values = $derived((r?.daily ?? []).map((d) => d.by_tool))
  const topSessions = $derived([...(sessions?.sessions ?? [])].sort((a, b) => b.totals.cost_usd - a.totals.cost_usd).slice(0, 5))
  const projectLabel = (name: string, hidden: boolean, id: string | number | null) =>
    hideAll || hidden ? `${t('projects.hidden')}${id !== null ? ` #${id}` : ''}` : name || t('projects.noProject')

  const days = $derived(r ? r.days_in_range : 7)
  function value(p: Provider) {
    const planId = s?.plans[p]
    const plan = planId ? plans?.providers[p]?.plans.find((x) => x.id === planId) : undefined
    const own = s?.plan_prices?.[p]
    const price = typeof own === 'number' && own > 0 ? own : plan?.monthly_usd ?? null
    const cost = (r?.by_tool ?? []).filter((g) => providerOfSource(g.key) === p).reduce((a, g) => a + g.totals.cost_usd, 0)
    const share = typeof price === 'number' && price > 0 ? (price * days) / (365.25 / 12) : null
    return { plan, cost, share, ratio: share ? cost / share : null }
  }
  const providers = $derived(
    providersOf((r?.by_tool ?? []).map((g) => g.key)),
  )
  const currentLimits = $derived(limits.filter((l) => l.window === 'five_hour' || l.window === 'seven_day'))
</script>

<main class="page">
  <header>
    <div class="brand">
      <img src="/app-icon.png" alt="" width="28" height="28" />
      <div>
        <h1>{t('rep.title')}</h1>
        <p class="range">
          {from ? fmtDate(from, 'long') : ''} – {to ? fmtDate(to, 'long') : ''}
          {#if r}<span class="muted">{' · '}{t('rep.days', { n: r.days_in_range })}</span>{/if}
        </p>
      </div>
    </div>
    <p class="muted small">{t('rep.generated', { d: fmtDateTime(Date.now()) })}</p>
  </header>

  {#if failed}
    <p>{t('common.error', { e: failed })}</p>
  {:else if r}
    {#if r.totals.events === 0}
      <p class="muted">{t('rep.none')}</p>
    {:else}
      {@const ct = change(r.totals.total_tokens, r.previous.total_tokens)}
      {@const cc = change(r.totals.cost_usd, r.previous.cost_usd)}
      <section class="kpis block">
        <div class="kpi">
          <span class="lab">{t('rep.tokens')}</span>
          <b class="num">{fmtCompact(r.totals.total_tokens)}</b>
          <span class="muted small">{t('rep.requests', { n: fmtInt(r.totals.events) })}{#if ct !== null} · {ct >= 0 ? '+' : ''}{fmtPct(ct)} {t('rep.vsPrev')}{/if}</span>
        </div>
        <div class="kpi">
          <span class="lab">{t('rep.cost')}</span>
          <b class="num">{fmtMoney(r.totals.cost_usd)}</b>
          <span class="muted small">{#if cc !== null}{cc >= 0 ? '+' : ''}{fmtPct(cc)} {t('rep.vsPrev')}{/if}</span>
        </div>
        <div class="kpi">
          <span class="lab">{t('rep.activeDays')}</span>
          <b class="num">{r.active_days} / {r.days_in_range}</b>
          <span class="muted small">{r.peak_day ? `${t('overview.peakDay')}: ${fmtDate(r.peak_day.date, 'short')}` : ''}</span>
        </div>
        <div class="kpi">
          <span class="lab">{t('rep.cacheHit')}</span>
          <b class="num">{prompt > 0 ? fmtPct((r.totals.tokens.cache_read / prompt) * 100, 1) : '—'}</b>
          <span class="muted small">{t('rep.cacheSaved', { v: fmtMoney(r.totals.cache_savings_usd) })}</span>
        </div>
      </section>

      <section class="block">
        <h2>{t('rep.daily')}</h2>
        <Columns dates={r.daily.map((d) => d.date)} {values} {series} format={fmtCompact} height={190} ariaLabel={t('rep.daily')} />
      </section>

      <div class="two">
        <section class="block">
          <h2>{t('rep.byTool')}</h2>
          <table>
            <tbody>
              {#each r.by_tool as g (g.key)}
                <tr>
                  <th scope="row"><i class="sw" style="background:{toolColor[g.key]}"></i>{toolLabel(g.key)}</th>
                  <td class="num">{fmtCompact(g.totals.total_tokens)}</td>
                  <td class="num">{fmtMoney(g.totals.cost_usd)}</td>
                </tr>
              {/each}
            </tbody>
          </table>

          <h2 class="gap">{t('rep.value')}</h2>
          {#each providers as p (p)}
            {@const v = value(p)}
            <p class="line">
              <b>{t(`provider.${p}`)}</b>{#if v.plan}<span class="muted">&nbsp;· {v.plan.name}</span>{/if}<br />
              {#if v.share !== null}
                <span class="big num">{fmtDec(v.ratio ?? 0, (v.ratio ?? 0) < 10 ? 1 : 0)}×</span>
                <span class="muted small">{t('rep.valueLine', { c: fmtMoney(v.cost), n: days, s: fmtMoney(v.share) })}</span>
              {:else}
                <span class="muted small">{!v.plan ? t('rep.noPlan') : v.plan.id === 'api' ? t('value.api') : v.plan.monthly_usd === 0 ? t('value.free') : t('value.enterPrice')}</span>
              {/if}
            </p>
          {/each}
        </section>

        <section class="block">
          <h2>{t('rep.limits')}</h2>
          {#if currentLimits.length === 0}
            <p class="muted small">{t('rep.limitNone')}</p>
          {:else}
            <table>
              <tbody>
                {#each currentLimits as l (l.provider + l.window + l.source)}
                  <tr>
                    <th scope="row">{t(`provider.${l.provider}`)} · {windowLabel(l.window)}</th>
                    <td class="num">
                      {#if l.state === 'fresh' && l.used_pct !== null}{fmtLimit(l.used_pct, mode)}
                      {:else if l.state === 'behind' && l.used_pct !== null}?{:else}—{/if}
                    </td>
                  </tr>
                  {#if l.state === 'fresh' && l.forecast && (l.forecast.kind === 'fills' || l.forecast.kind === 'safe')}
                    <tr class="sub">
                      <td colspan="2" class="muted small">
                        {#if l.forecast.kind === 'fills' && l.forecast.fills_at_ms}{t('forecast.fillsShort', { t: fmtDateTime(l.forecast.fills_at_ms) })}
                        {:else if l.forecast.at_reset_pct !== null}{t('forecast.safe', { pct: fmtLimit(l.forecast.at_reset_pct, mode, true) })}{/if}
                      </td>
                    </tr>
                  {/if}
                {/each}
              </tbody>
            </table>
          {/if}

          {#if ctx && ctx.requests > 0}
            <h2 class="gap">{t('rep.context')}</h2>
            <p class="small">{t('rep.contextLine', { avg: fmtCompact(ctx.avg), p90: fmtCompact(ctx.p90), max: fmtCompact(ctx.max), extra: fmtMoney(ctx.long_context_extra_usd), n: fmtInt(ctx.long_context_requests) })}</p>
          {/if}
        </section>
      </div>

      <section class="block">
        <h2>{t('rep.models')}</h2>
        <table>
          <thead>
            <tr>
              <th scope="col">{t('common.model')}</th>
              <th scope="col" class="num">{t('metric.input')}</th>
              <th scope="col" class="num">{t('metric.output')}</th>
              <th scope="col" class="num">{t('metric.cacheRead')}</th>
              <th scope="col" class="num">{t('metric.apiEq')}</th>
              <th scope="col" class="num">{t('rep.share')}</th>
            </tr>
          </thead>
          <tbody>
            {#each r.by_model.slice(0, 8) as g (g.key)}
              <tr>
                <th scope="row">{g.label}</th>
                <td class="num">{fmtCompact(g.totals.tokens.input)}</td>
                <td class="num">{fmtCompact(g.totals.tokens.output)}</td>
                <td class="num">{fmtCompact(g.totals.tokens.cache_read)}</td>
                <td class="num">{g.totals.unpriced_events > 0 ? '—' : fmtMoney(g.totals.cost_usd)}</td>
                <td class="num">{r.totals.cost_usd > 0 && g.totals.unpriced_events === 0 ? fmtPct((g.totals.cost_usd / r.totals.cost_usd) * 100, 1) : '—'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>

      <div class="two">
        <section class="block">
          <h2>{t('rep.projects')}</h2>
          <table>
            <tbody>
              {#each r.by_project.slice(0, 6) as g (g.key)}
                <tr>
                  <th scope="row" class="ell">{g.key === 'none' ? t('projects.noProject') : projectLabel(g.label, g.hidden, g.key)}</th>
                  <td class="num">{fmtCompact(g.totals.total_tokens)}</td>
                  <td class="num">{fmtMoney(g.totals.cost_usd)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
        <section class="block">
          <h2>{t('rep.sessions')}</h2>
          <table>
            <tbody>
              {#each topSessions as x (x.tool + x.session_id)}
                <tr>
                  <th scope="row" class="ell">
                    <i class="sw" style="background:{toolColor[x.tool]}"></i>{fmtDateTime(x.started_ms)}
                    <span class="muted">{' · '}{projectLabel(x.project, x.hidden, x.project_id)}</span>
                  </th>
                  <td class="num">{fmtSpan(x.ended_ms - x.started_ms)}</td>
                  <td class="num">{x.totals.unpriced_events === x.totals.events ? '—' : fmtMoney(x.totals.cost_usd)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
      </div>

      <section class="block notes">
        <h2>{t('rep.notes')}</h2>
        <p class="small muted">{t('rep.note1')}</p>
        <p class="small muted">{t('rep.note2')}</p>
        {#if r.unpriced_models.length}<p class="small muted">{t('rep.note3', { m: r.unpriced_models.join(', ') })}</p>{/if}
        <p class="small muted">AI Usage Tracker · {i18n.lang === 'tr' ? 'yerel rapor' : 'local report'}</p>
      </section>
    {/if}
  {/if}
</main>

<style>
  /* undo the app shell's fixed-height, overflow-hidden root so the report flows across pages */
  :global(html),
  :global(body),
  :global(#app) {
    background: #fff;
    margin: 0;
    height: auto;
    overflow: visible;
  }
  .page {
    /* the printable width of A4 with 12 mm margins */
    width: 698px;
    margin: 0 auto;
    padding: 4px 0;
    color: var(--ink);
    font-size: 12.5px;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    border-bottom: 1px solid var(--hairline-strong);
    padding-bottom: 12px;
    margin-bottom: 14px;
  }
  .brand {
    display: flex;
    gap: 12px;
    align-items: center;
  }
  h1 {
    font-size: 22px;
    margin: 0;
    letter-spacing: -0.02em;
  }
  .range {
    margin: 2px 0 0;
    font-size: 13px;
  }
  h2 {
    font-size: 13px;
    font-weight: 650;
    margin: 0 0 8px;
  }
  h2.gap {
    margin-top: 16px;
  }
  .block {
    break-inside: avoid;
    margin-bottom: 18px;
  }
  .kpis {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
  }
  .kpi {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 12px;
    border: 1px solid var(--hairline-strong);
    border-radius: 10px;
  }
  .lab {
    font-size: 11px;
    color: var(--ink-2);
  }
  .kpi b {
    font-size: 20px;
    letter-spacing: -0.02em;
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 22px;
  }
  .two > section {
    min-width: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    padding: 4px 6px;
    border-bottom: 0.5px solid var(--hairline);
    text-align: left;
    font-weight: 400;
  }
  thead th {
    font-size: 11px;
    color: var(--ink-2);
  }
  tbody th {
    font-weight: 500;
  }
  tr.sub td {
    border-bottom: 0.5px solid var(--hairline);
    padding-top: 0;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .ell {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sw {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-right: 6px;
  }
  .line {
    margin: 0 0 8px;
  }
  .big {
    font-size: 16px;
    font-weight: 700;
    margin-right: 6px;
  }
  .muted {
    color: var(--ink-2);
  }
  .small {
    font-size: 11px;
  }
  .notes p {
    margin: 0 0 4px;
  }
  @page {
    size: A4;
  }
</style>
