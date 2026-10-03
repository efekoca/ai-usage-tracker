<script lang="ts">
  // Days: the period day by day, day patterns, and everything about one chosen day.
  import { app, toolColor } from '../lib/store.svelte'
  import { api, type DayDetail, type DayPoint } from '../lib/api'
  import { fmtClock, fmtCompact, fmtDate, fmtHour, fmtInt, fmtMoney, fmtPct, t, toolLabel, weekdayNames, windowLabel } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import StatTile from '../components/StatTile.svelte'
  import Icon from '../components/Icon.svelte'
  import CalendarHeatmap from '../components/charts/CalendarHeatmap.svelte'
  import Columns from '../components/charts/Columns.svelte'
  import BarList from '../components/charts/BarList.svelte'

  type Metric = 'tokens' | 'cost' | 'events'
  let mode: 'calendar' | 'table' = $state('calendar')
  let metric = $state<Metric>(app.settings?.primary_metric ?? 'tokens')
  let picked: string | null = $state(null)

  const r = $derived(app.report)
  const days = $derived(r?.daily ?? [])
  const active = $derived(days.filter((d) => d.events > 0))
  const fmt = $derived(metric === 'tokens' ? fmtCompact : metric === 'cost' ? (v: number) => fmtMoney(v) : fmtInt)
  const of = (d: DayPoint) => (metric === 'tokens' ? d.tokens : metric === 'cost' ? d.cost_usd : d.events)
  const tools = $derived([...new Set(days.flatMap((d) => Object.keys(d.by_tool)))].sort())
  const rows = $derived([...days].reverse())
  // a calendar is sparse for short periods; columns read better there
  const short = $derived(days.length <= 45)
  const colSeries = $derived(
    metric === 'events' ? [{ key: 'events', label: t('days.metric.events'), color: 'var(--s1)' }] : tools.map((k) => ({ key: k, label: toolLabel(k), color: toolColor[k] })),
  )
  const colValues = $derived(days.map((d) => (metric === 'tokens' ? d.by_tool : metric === 'cost' ? d.cost_by_tool : { events: d.events })))

  // the chosen day: the one picked, otherwise the latest day with use
  const selected = $derived(picked && days.some((d) => d.date === picked) ? picked : (active.at(-1)?.date ?? null))
  let detail = $state<DayDetail | null>(null)
  $effect(() => {
    void app.tick
    const d = selected
    if (!d) {
      detail = null
      return
    }
    api.dayDetail(d, $state.snapshot(app.filter)).then((x) => {
      if (x.date === selected) detail = x
    })
  })
  const idx = $derived(active.findIndex((d) => d.date === selected))
  const prevDay = $derived(idx > 0 ? active[idx - 1].date : idx === -1 ? (active.filter((d) => d.date < (selected ?? '')).at(-1)?.date ?? null) : null)
  const nextDay = $derived(idx >= 0 && idx < active.length - 1 ? active[idx + 1].date : idx === -1 ? (active.find((d) => d.date > (selected ?? ''))?.date ?? null) : null)

  // ---- patterns over the period
  const busiest = $derived(active.length ? active.reduce((a, b) => (of(b) > of(a) ? b : a)) : null)
  const perActive = $derived(active.length ? active.reduce((s, d) => s + of(d), 0) / active.length : 0)
  const streaks = $derived.by(() => {
    let best = 0
    let run = 0
    for (const d of days) {
      run = d.events > 0 ? run + 1 : 0
      best = Math.max(best, run)
    }
    // today without use yet does not break the current streak
    let now = 0
    const list = days.at(-1)?.events === 0 ? days.slice(0, -1) : days
    for (let i = list.length - 1; i >= 0 && list[i].events > 0; i--) now++
    return { best, now }
  })
  const dow = (date: string) => (new Date(date + 'T00:00:00').getDay() + 6) % 7 // Monday = 0
  const weekendShare = $derived.by(() => {
    const total = days.reduce((s, d) => s + of(d), 0)
    return total > 0 ? (days.filter((d) => dow(d.date) >= 5).reduce((s, d) => s + of(d), 0) / total) * 100 : null
  })
  const weekdays = $derived.by(() => {
    const sum = Array(7).fill(0)
    const n = Array(7).fill(0)
    for (const d of days) {
      sum[dow(d.date)] += of(d)
      n[dow(d.date)]++
    }
    return weekdayNames().map((label, i) => ({ label, value: n[i] ? sum[i] / n[i] : 0, n: n[i] }))
  })
  const weekdayTop = $derived(Math.max(1e-9, ...weekdays.map((w) => w.value)))

  // ---- the chosen day
  const dv = (x: { tokens: number; cost_usd: number; events: number }) => (metric === 'tokens' ? x.tokens : metric === 'cost' ? x.cost_usd : x.events)
  const hourKeys = Array.from({ length: 24 }, (_, h) => String(h))
  const hourValues = $derived(
    (detail?.hourly ?? []).map((h) => (metric === 'tokens' ? h.by_tool : metric === 'cost' ? { cost: h.cost_usd } : { events: h.events })),
  )
  const hourSeries = $derived(
    metric === 'tokens'
      ? [...new Set((detail?.hourly ?? []).flatMap((h) => Object.keys(h.by_tool)))].sort().map((k) => ({ key: k, label: toolLabel(k), color: toolColor[k] }))
      : [{ key: metric === 'cost' ? 'cost' : 'events', label: metric === 'cost' ? t('metric.apiEq') : t('days.metric.events'), color: 'var(--s1)' }],
  )
  const gv = (g: { totals: { total_tokens: number; cost_usd: number; events: number } }) =>
    metric === 'tokens' ? g.totals.total_tokens : metric === 'cost' ? g.totals.cost_usd : g.totals.events
  const cacheHit = (x: { input: number; cache_read: number; cache_write: number }) => {
    const prompt = x.input + x.cache_read + x.cache_write
    return prompt > 0 ? (x.cache_read / prompt) * 100 : null
  }
  const projectName = (g: { key: string; label: string; hidden: boolean }) =>
    g.key === 'none' ? t('projects.noProject') : g.hidden ? `${t('projects.hidden')} #${g.key}` : g.label
</script>

<div class="head">
  <Segmented label={t('daily.title')} options={[{ value: 'calendar', label: t('common.chart') }, { value: 'table', label: t('daily.table') }]} bind:value={mode} />
  <Segmented
    label={t('metric.tokens')}
    options={[{ value: 'tokens', label: t('metric.tokens') }, { value: 'cost', label: t('metric.cost') }, { value: 'events', label: t('days.metric.events') }]}
    bind:value={metric}
  />
</div>

{#if !r}
  <p class="muted">{t('common.loading')}</p>
{:else if mode === 'calendar'}
  <section class="tiles card">
    <StatTile label={t('days.peak')} value={busiest ? fmt(of(busiest)) : '—'}>
      {#if busiest}<button class="link subtle" onclick={() => (picked = busiest!.date)}>{fmtDate(busiest.date, 'medium')}</button>{/if}
    </StatTile>
    <StatTile label={t('days.perActive')} value={fmt(perActive)}>
      <span class="subtle">{t('days.perActiveSub', { n: active.length })}</span>
    </StatTile>
    <StatTile label={t('days.streak')} value={t('days.streakValue', { n: streaks.best })}>
      <span class="subtle">{t('days.streakNow', { n: streaks.now })}</span>
    </StatTile>
    <StatTile label={t('days.weekend')} value={weekendShare === null ? '—' : fmtPct(weekendShare, 1)}>
      <span class="subtle">{t('days.weekendSub')}</span>
    </StatTile>
  </section>

  <section class="card">
    {#if short}
      <Columns
        dates={days.map((d) => d.date)}
        values={colValues}
        series={colSeries}
        format={metric === 'tokens' ? fmtCompact : metric === 'cost' ? (v) => fmtMoney(v, { compact: true }) : fmtInt}
        ariaLabel={t('daily.calendar')}
        {selected}
        onselect={(d) => (picked = d)}
      />
    {:else}
      <CalendarHeatmap {days} value={(i) => of(days[i])} format={fmt} ariaLabel={t('daily.calendar')} {selected} onselect={(d) => (picked = d)} />
    {/if}
  </section>

  <div class="grid">
    <section class="card day">
      {#if !selected}
        <p class="muted">{t('days.pick')}</p>
      {:else}
        <div class="dayhead">
          <button class="btn ghost icon" disabled={!prevDay} aria-label={t('days.prev')} title={t('days.prev')} onclick={() => (picked = prevDay)}><Icon name="chevron" size={16} /></button>
          <h2>{fmtDate(selected, 'long')}</h2>
          <button class="btn ghost icon next" disabled={!nextDay} aria-label={t('days.next')} title={t('days.next')} onclick={() => (picked = nextDay)}><Icon name="chevron" size={16} /></button>
        </div>
        {#if detail && detail.date === selected}
          {#if detail.totals.events === 0}
            <p class="muted">{t('daily.noData')}</p>
          {:else}
            <div class="facts">
              <div><span class="muted small">{t('metric.tokens')}</span><b class="big">{fmtCompact(detail.totals.total_tokens)}</b></div>
              <div><span class="muted small">{t('metric.apiEq')}</span><b class="big">{fmtMoney(detail.totals.cost_usd)}</b></div>
              <div><span class="muted small">{t('days.metric.events')}</span><b class="big">{fmtInt(detail.totals.events)}</b></div>
              <div><span class="muted small">{t('days.sessions')}</span><b class="big">{fmtInt(detail.sessions)}</b></div>
              <div>
                <span class="muted small">{t('days.activeHours')}</span>
                <b class="big">{detail.first_ms && detail.last_ms ? `${fmtClock(detail.first_ms)}–${fmtClock(detail.last_ms)}` : '—'}</b>
              </div>
              <div>
                <span class="muted small">{t('days.cacheHit')}</span>
                <b class="big">{cacheHit(detail.totals.tokens) === null ? '—' : fmtPct(cacheHit(detail.totals.tokens)!, 1)}</b>
              </div>
            </div>

            <h3>{t('days.hourly')}</h3>
            <Columns
              dates={hourKeys}
              values={hourValues}
              series={hourSeries}
              height={170}
              format={metric === 'tokens' ? fmtCompact : metric === 'cost' ? (v) => fmtMoney(v, { compact: true }) : fmtInt}
              ariaLabel={t('days.hourly')}
              xLabel={(k) => fmtHour(Number(k))}
              labelStep={3}
              tipLabel={(k) => `${fmtHour(Number(k))}–${fmtHour((Number(k) + 1) % 24)}`}
            />

            <div class="lists">
              <div>
                <h3>{t('days.tools')}</h3>
                <BarList ariaLabel={t('days.tools')} items={detail.by_tool.map((g) => ({ key: g.key, label: toolLabel(g.key), value: gv(g), color: toolColor[g.key] }))} format={fmt} max={4} />
              </div>
              <div>
                <h3>{t('days.models')}</h3>
                <BarList ariaLabel={t('days.models')} items={detail.by_model.map((g) => ({ key: g.key, label: g.label, value: gv(g) }))} format={fmt} max={5} />
              </div>
              <div>
                <h3>{t('days.projects')}</h3>
                <BarList ariaLabel={t('days.projects')} items={detail.by_project.map((g) => ({ key: g.key, label: projectName(g), value: gv(g), muted: g.key === 'none' }))} format={fmt} max={5} />
              </div>
            </div>

            <h3>{t('days.limits')}</h3>
            {#if detail.limit_peaks.length === 0}
              <p class="subtle small">{t('days.limitsNone')}</p>
            {:else}
              <div class="peaks">
                {#each detail.limit_peaks as p (p.provider + p.window)}
                  <span class="peak" class:full={p.peak_pct >= 99.5}>
                    {t(`provider.${p.provider}`)} · {windowLabel(p.window)} <b>{fmtPct(p.peak_pct)}</b><span class="subtle"> · {fmtClock(p.at_ms)}</span>
                  </span>
                {/each}
              </div>
            {/if}
          {/if}
        {:else}
          <p class="muted">{t('common.loading')}</p>
        {/if}
      {/if}
    </section>

    <div class="side">
      <section class="card">
        <h2>{t('days.weekdays')}</h2>
        <p class="subtle small help">{t('days.weekdaysHelp')}</p>
        <ul class="wd" aria-label={t('days.weekdays')}>
          {#each weekdays as w (w.label)}
            <li>
              <span class="lbl">{w.label}</span>
              <span class="track" aria-hidden="true"><span class="fill" style="width:{(w.value / weekdayTop) * 100}%"></span></span>
              <span class="num">{w.n ? fmt(w.value) : '—'}</span>
            </li>
          {/each}
        </ul>
      </section>
      <section class="card">
        <h2>{t('days.top')}</h2>
        <BarList ariaLabel={t('days.top')} items={active.map((d) => ({ key: d.date, label: fmtDate(d.date, 'medium'), value: of(d) }))} format={fmt} max={6} onpick={(k) => (picked = k)} />
      </section>
    </div>
  </div>
{:else}
  <section class="card table-card">
    <table>
      <thead>
        <tr>
          <th scope="col">{t('daily.date')}</th>
          {#each tools as tl (tl)}<th scope="col" class="num">{toolLabel(tl)}</th>{/each}
          <th scope="col" class="num">{t('metric.total')}</th>
          <th scope="col" class="num">{t('metric.apiEq')}</th>
          <th scope="col" class="num">{t('days.col.requests')}</th>
          <th scope="col" class="num">{t('days.col.cacheHit')}</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as d (d.date)}
          {@const hit = d.prompt_tokens > 0 ? (d.cache_read / d.prompt_tokens) * 100 : null}
          <tr class:zero={d.events === 0}>
            <th scope="row"><button class="link" disabled={d.events === 0} onclick={() => { picked = d.date; mode = 'calendar' }}>{fmtDate(d.date, 'medium')}</button></th>
            {#each tools as tl (tl)}<td class="num">{d.by_tool[tl] ? fmtCompact(d.by_tool[tl]) : '—'}</td>{/each}
            <td class="num">{fmtCompact(d.tokens)}</td>
            <td class="num">{fmtMoney(d.cost_usd)}</td>
            <td class="num">{fmtInt(d.events)}</td>
            <td class="num">{hit === null ? '—' : fmtPct(hit, 1)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>
{/if}

<style>
  .head {
    display: flex;
    gap: 12px;
    margin-bottom: 16px;
    flex-wrap: wrap;
  }
  section {
    margin-bottom: 16px;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 0;
    padding: 0;
  }
  @media (max-width: 1000px) {
    .tiles {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(280px, 1fr);
    gap: 16px;
    align-items: start;
  }
  @media (max-width: 1100px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .side section {
    margin-bottom: 16px;
  }
  .dayhead {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 14px;
  }
  .dayhead h2 {
    margin: 0;
    flex: 1;
    text-align: center;
  }
  .icon {
    padding: 4px 6px;
  }
  .icon:not(.next) :global(svg) {
    transform: rotate(180deg);
  }
  .facts {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 14px 24px;
    margin-bottom: 18px;
  }
  .facts div {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .big {
    font-size: 20px;
    font-weight: 650;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  h3 {
    font-size: 13px;
    font-weight: 600;
    margin: 14px 0 8px;
    color: var(--ink-2);
  }
  .lists {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 18px;
  }
  @media (max-width: 760px) {
    .lists {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .peaks {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .peak {
    font-size: 12.5px;
    padding: 5px 10px;
    border-radius: 8px;
    background: var(--surface-hover);
  }
  .peak.full b {
    color: var(--bad-ink);
  }
  .help {
    margin: 2px 0 10px;
  }
  .wd {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .wd li {
    display: grid;
    grid-template-columns: 44px 1fr 80px;
    align-items: center;
    gap: 10px;
    font-size: 13px;
  }
  .wd .track {
    height: 8px;
    border-radius: 4px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .wd .fill {
    display: block;
    height: 100%;
    border-radius: 4px;
    background: var(--s1);
  }
  .link {
    border: 0;
    background: transparent;
    padding: 0;
    font: inherit;
    color: inherit;
    cursor: pointer;
    text-align: left;
  }
  .link:hover:not(:disabled) {
    text-decoration: underline;
  }
  .link:disabled {
    cursor: default;
  }
  .table-card {
    padding: 6px 0;
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 8px 20px;
    text-align: left;
    border-bottom: 0.5px solid var(--hairline);
    white-space: nowrap;
  }
  thead th {
    color: var(--ink-2);
    font-weight: 500;
    font-size: 12px;
    position: sticky;
    top: 0;
    background: var(--surface);
  }
  tbody th {
    font-weight: 500;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  tr.zero td,
  tr.zero th {
    color: var(--ink-3);
  }
  tbody tr:last-child td,
  tbody tr:last-child th {
    border-bottom: 0;
  }
</style>
