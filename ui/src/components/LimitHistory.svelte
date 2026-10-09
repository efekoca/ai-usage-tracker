<script lang="ts">
  import { api, providersOf, type LimitHistoryView, type Provider, type WindowRecord } from '../lib/api'
  import { app, latest, markTint } from '../lib/store.svelte'
  import { fmtCompact, fmtDateTime, fmtDuration, fmtInt, fmtMoney, fmtPct, i18n, limitName, t, windowLabel } from '../lib/i18n.svelte'
  import Segmented from './Segmented.svelte'
  import Select from './Select.svelte'
  import StatTile from './StatTile.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'
  import Icon from './Icon.svelte'
  import BrandIcon from './BrandIcon.svelte'
  import BarValue from './BarValue.svelte'
  import WindowHistory from './charts/WindowHistory.svelte'

  const DAY = 864e5
  let provider: 'all' | Provider = $state('all')
  let win: 'five_hour' | 'seven_day' = $state('five_hour')
  let range = $state(28)
  let status: 'all' | 'full' | 'high' | 'complete' = $state('all')
  let sortKey: 'start' | 'peak' | 'toFull' | 'local' = $state('start')
  let sortDesc = $state(true)
  let showAll = $state(false)

  let view = $state<LimitHistoryView | null>(null)
  let failed = $state('')
  $effect(() => {
    void app.tick
    const days = range
    return latest(
      () => api.limitHistory(days),
      (v) => {
        view = v
        failed = ''
      },
      (e) => {
        view = null
        failed = String(e)
      },
    )
  })

  const enabled = $derived(providersOf(app.settings?.enabled_sources ?? []).filter((p) => (view?.advice ?? []).some((a) => a.provider === p) || (view?.history.series ?? []).some((x) => x.provider === p)))
  const providers = $derived(provider === 'all' ? enabled : [provider])
  const series = $derived((view?.history.series ?? []).filter((s) => s.window === win && providers.includes(s.provider)))
  type Row = WindowRecord & { provider: Provider; limit_id: string }
  const startOf = (w: WindowRecord) => w.start_ms ?? w.first_ms
  const toFull = (w: WindowRecord) => (w.full_at_ms !== null && w.start_ms !== null ? w.full_at_ms - w.start_ms : null)
  const all = $derived<Row[]>(series.flatMap((s) => s.windows.map((w) => ({ ...w, provider: s.provider, limit_id: s.limit_id }))))
  const rows = $derived.by(() => {
    const kept = all.filter((w) =>
      status === 'full' ? w.full : status === 'high' ? w.peak_pct >= 80 : status === 'complete' ? w.complete && !w.in_progress : true,
    )
    const key = (w: Row) => (sortKey === 'peak' ? w.peak_pct : sortKey === 'toFull' ? (toFull(w) ?? -1) : sortKey === 'local' ? w.local.cost_usd : startOf(w))
    return kept.sort((a, b) => (sortDesc ? key(b) - key(a) : key(a) - key(b)))
  })
  const visible = $derived(showAll ? rows : rows.slice(0, 30))
  const keyOf = (p: Provider, id: string, w: WindowRecord) => `${p}:${id}:${w.first_ms}:${w.resets_at_ms ?? 0}`
  const kept = $derived(new Set(rows.map((w) => keyOf(w.provider, w.limit_id, w))))
  // range >= 3650 means "all": start at the first window shown
  const chartFrom = $derived(
    range >= 3650 && all.length ? Math.min(...all.map(startOf)) : (view?.history.to_ms ?? Date.now()) - range * DAY,
  )

  const stats = $derived.by(() => {
    const peaks = rows.map((w) => w.peak_pct).sort((a, b) => a - b)
    const full = rows.filter((w) => w.full)
    const fills = rows.map(toFull).filter((x): x is number => x !== null)
    const n = peaks.length
    return {
      n,
      full: full.length,
      fullMinutes: full.reduce((s, w) => s + (w.full_minutes ?? 0), 0),
      avg: n ? peaks.reduce((s, p) => s + p, 0) / n : null,
      median: n ? (n % 2 ? peaks[(n - 1) / 2] : (peaks[n / 2 - 1] + peaks[n / 2]) / 2) : null,
      max: n ? peaks[n - 1] : null,
      toFull: fills.length ? fills.reduce((s, x) => s + x, 0) / fills.length : null,
    }
  })
  const buckets = $derived.by(() => {
    const edges = [
      { label: '1–20', test: (p: number) => p < 20 },
      { label: '20–40', test: (p: number) => p >= 20 && p < 40 },
      { label: '40–60', test: (p: number) => p >= 40 && p < 60 },
      { label: '60–80', test: (p: number) => p >= 60 && p < 80 },
      { label: '80–99', test: (p: number) => p >= 80 && p < 99.5 },
      { label: t('history.dist.full'), test: (p: number) => p >= 99.5, full: true },
    ]
    return edges.map((e) => ({ ...e, n: rows.filter((w) => e.test(w.peak_pct)).length }))
  })
  const bucketTop = $derived(Math.max(1, ...buckets.map((b) => b.n)))

  function sortBy(k: typeof sortKey) {
    if (sortKey === k) sortDesc = !sortDesc
    else {
      sortKey = k
      sortDesc = true
    }
  }
  const stateOf = (w: WindowRecord) => (w.full ? 'full' : w.in_progress ? 'running' : w.complete ? 'complete' : 'partial')
  const early = (w: WindowRecord) => w.end_ms !== null && w.resets_at_ms !== null && w.end_ms < w.resets_at_ms
  const ranges = [7, 28, 56, 90, 365, 3650]
</script>

<section class="card filters">
  {#if enabled.length > 1}
    <div class="f">
      <span class="small muted">{t('history.filter.provider')}</span>
      <Segmented label={t('history.filter.provider')} bind:value={provider} options={[{ value: 'all', label: t('history.filter.all') }, ...enabled.map((p) => ({ value: p, label: t(`provider.${p}`) }))]} />
    </div>
  {/if}
  <div class="f">
    <span class="small muted">{t('history.filter.window')}</span>
    <Segmented label={t('history.filter.window')} bind:value={win} options={[{ value: 'five_hour', label: windowLabel('five_hour') }, { value: 'seven_day', label: windowLabel('seven_day') }]} />
  </div>
  <div class="f">
    <span class="small muted">{t('history.filter.range')}</span>
    <Select label={t('history.filter.range')} value={String(range)} minWidth={150} options={ranges.map((d) => ({ value: String(d), label: t(`history.range.${d}`) }))} onchange={(v) => (range = Number(v))} />
  </div>
  <div class="f">
    <span class="small muted">{t('history.filter.status')}</span>
    <Select
      label={t('history.filter.status')}
      value={status}
      minWidth={190}
      options={[{ value: 'all', label: t('history.status.all') }, { value: 'full', label: t('history.status.full') }, { value: 'high', label: t('history.status.high') }, { value: 'complete', label: t('history.status.complete') }]}
      onchange={(v) => (status = v as typeof status)}
    />
  </div>
</section>
<p class="subtle small lead">{t('history.lead')}</p>

{#if view}
  <section class="tiles card">
    <StatTile label={t('history.stat.windows')} value={fmtInt(stats.n)} />
    <StatTile label={t('history.stat.full')} value={fmtInt(stats.full)}>
      {#if stats.fullMinutes > 0}<span class="subtle">{t('history.stat.fullSub', { d: fmtDuration(stats.fullMinutes * 60000) })}</span>{/if}
    </StatTile>
    <StatTile label={t('history.stat.peak')} value={stats.avg === null ? '—' : fmtPct(stats.avg)}>
      {#if stats.median !== null}<span class="subtle">{t('history.stat.peakSub', { m: fmtPct(stats.median), max: fmtPct(stats.max ?? 0) })}</span>{/if}
    </StatTile>
    <StatTile label={t('history.stat.toFull')} value={stats.toFull === null ? '—' : fmtDuration(stats.toFull)}>
      <span class="subtle">{t('history.stat.toFullSub')}</span>
    </StatTile>
  </section>

  {#if series.some((s) => s.capacity)}
    <section class="card capacity">
      {#each series.filter((s) => s.capacity) as s (s.provider + s.limit_id)}
        {@const c = s.capacity!}
        <div class="cap" title={t('history.stat.capacityHelp')}>
          <div class="small muted">{t('history.stat.capacity', { p: limitName(s.provider, s.limit_id) })} · {windowLabel(s.window)}</div>
          <div class="capv">≈ {fmtMoney(c.median_usd)} <AccuracyBadge kind="estimated" compact /></div>
          <div class="subtle small">{t('history.stat.capacitySub', { n: c.windows, min: fmtMoney(c.min_usd), max: fmtMoney(c.max_usd) })}</div>
        </div>
      {/each}
      <p class="subtle small capnote"><Icon name="info" size={13} /> {t('history.stat.capacityHelp')}</p>
    </section>
  {/if}

  <div class="grid">
    <section class="card">
      <div class="legend" aria-hidden="true">
        <span><i class="sw complete"></i>{t('history.legend.complete')}</span>
        <span><i class="sw partial"></i>{t('history.legend.partial')}</span>
        <span><i class="sw full"></i>{t('history.legend.full')}</span>
        <span><i class="sw running"></i>{t('history.legend.running')}</span>
      </div>
      {#if series.length === 0}
        <p class="muted empty">{t('history.empty')}</p>
      {/if}
      {#each series as s (s.provider + s.limit_id)}
        {@const shownW = s.windows.filter((w) => kept.has(keyOf(s.provider, s.limit_id, w)))}
        <div class="chart">
          {#if series.length > 1}<h3 class="brandname"><BrandIcon provider={s.provider} color={markTint(s.provider)} />{limitName(s.provider, s.limit_id)}</h3>{/if}
          <WindowHistory
            windows={shownW}
            fromMs={chartFrom}
            toMs={view.history.to_ms}
            height={190}
            ariaLabel="{limitName(s.provider, s.limit_id)} · {windowLabel(win)}"
          />
        </div>
      {/each}
    </section>
    <section class="card">
      <h2>{t('history.dist')}</h2>
      <ul class="dist" aria-label={t('history.dist')}>
        {#each buckets as b (b.label)}
          <li>
            <span class="lbl">{b.full ? b.label : i18n.lang === 'tr' ? `%${b.label}` : `${b.label} %`}</span>
            <span class="track" aria-hidden="true"><span class="fill" class:full={b.full} style="width:{(b.n / bucketTop) * 100}%"></span></span>
            <span class="num">{fmtInt(b.n)}</span>
          </li>
        {/each}
      </ul>
    </section>
  </div>

  <section class="card table-card">
    <h2 class="pad">{t('history.table')}</h2>
    {#if rows.length === 0}
      <p class="muted pad empty">{t('history.none')}</p>
    {:else}
      <table>
        <thead>
          <tr>
            {#if providers.length > 1}<th scope="col">{t('history.col.provider')}</th>{/if}
            <th scope="col"><button class="sort" onclick={() => sortBy('start')}>{t('history.col.start')}{sortKey === 'start' ? (sortDesc ? ' ↓' : ' ↑') : ''}</button></th>
            <th scope="col">{t('history.col.end')}</th>
            <th scope="col" class="num"><button class="sort" onclick={() => sortBy('peak')}>{t('history.col.peak')}{sortKey === 'peak' ? (sortDesc ? ' ↓' : ' ↑') : ''}</button></th>
            <th scope="col">{t('history.col.status')}</th>
            <th scope="col" class="num"><button class="sort" onclick={() => sortBy('toFull')}>{t('history.col.toFull')}{sortKey === 'toFull' ? (sortDesc ? ' ↓' : ' ↑') : ''}</button></th>
            <th scope="col" class="num">{t('history.col.fullFor')}</th>
            <th scope="col" class="num" title={t('history.localHelp')}><button class="sort" onclick={() => sortBy('local')}>{t('history.col.local')}{sortKey === 'local' ? (sortDesc ? ' ↓' : ' ↑') : ''}</button></th>
            <th scope="col" class="num opt">{t('history.col.readings')}</th>
          </tr>
        </thead>
        <tbody>
          {#each visible as w (keyOf(w.provider, w.limit_id, w))}
            {@const st = stateOf(w)}
            <tr>
              {#if providers.length > 1}<td><span class="brandname"><BrandIcon provider={w.provider} size={13} color={markTint(w.provider)} />{limitName(w.provider, w.limit_id)}</span></td>{/if}
              <td>{fmtDateTime(startOf(w))}</td>
              <td>
                {fmtDateTime(w.end_ms ?? w.resets_at_ms ?? w.last_ms)}
                {#if early(w)}<span class="subtle small">{' · '}{t('history.state.early')}</span>{/if}
              </td>
              <td class="num">
                <BarValue pct={w.peak_pct} tone={w.full ? 'critical' : 'series'} text="{w.complete || w.full || w.in_progress ? '' : '≥ '}{fmtPct(w.peak_pct)}" />
              </td>
              <td><span class="state {st}"><i></i>{t(`history.state.${st}`)}</span></td>
              <td class="num">{toFull(w) === null ? '—' : fmtDuration(toFull(w)!)}</td>
              <td class="num">{w.full_minutes ? fmtDuration(w.full_minutes * 60000) : '—'}</td>
              <td class="num">
                {#if w.local.requests === 0}—{:else}{fmtCompact(w.local.tokens)} <span class="subtle">· {fmtMoney(w.local.cost_usd)}</span>{/if}
              </td>
              <td class="num opt">{fmtInt(w.readings)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if rows.length > 30}
        <div class="pad foot"><button class="btn ghost small" onclick={() => (showAll = !showAll)}>{showAll ? t('tools.showLess') : t('tools.showAll', { n: rows.length })}</button></div>
      {/if}
    {/if}
  </section>
{:else if failed}
  <div class="banner"><Icon name="warning" size={16} />{t('common.error', { e: failed })}</div>
{/if}

<style>
  section {
    margin-bottom: 16px;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 14px 24px;
    align-items: flex-end;
  }
  .f {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lead {
    margin: -6px 0 14px;
    max-width: 820px;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 24px;
  }
  @container main (max-width: 940px) {
    .tiles {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .capacity {
    display: flex;
    flex-wrap: wrap;
    gap: 12px 40px;
    align-items: flex-start;
  }
  .capv {
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.02em;
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 2px 0;
  }
  .capnote {
    flex-basis: 100%;
    margin: 0;
    display: flex;
    gap: 6px;
    align-items: flex-start;
    max-width: 820px;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(260px, 1fr);
    gap: 16px;
    align-items: start;
    margin-bottom: 16px;
  }
  @container main (max-width: 1040px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .grid section {
    margin: 0;
  }
  .chart + .chart {
    margin-top: 18px;
  }
  h3 {
    font-size: 13px;
    font-weight: 600;
    margin: 0 0 6px;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    font-size: 12px;
    color: var(--ink-2);
    margin-bottom: 12px;
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .sw {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    display: inline-block;
    background: var(--s1);
  }
  .sw.partial {
    background: repeating-linear-gradient(45deg, var(--s1) 0 2px, var(--surface) 2px 4px);
    box-shadow: inset 0 0 0 1px var(--s1);
  }
  .sw.full {
    background: var(--critical);
  }
  .sw.running {
    opacity: 0.55;
  }
  .empty {
    padding: 24px 0;
  }
  .dist {
    list-style: none;
    margin: 10px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .dist li {
    display: grid;
    grid-template-columns: 64px 1fr 40px;
    gap: 10px;
    align-items: center;
    font-size: 13px;
  }
  .dist .track {
    height: 10px;
    border-radius: 4px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .dist .fill {
    display: block;
    height: 100%;
    border-radius: 4px;
    background: var(--s1);
  }
  .dist .fill.full {
    background: var(--critical);
  }
  .table-card {
    padding: 6px 0;
    overflow-x: auto;
  }
  .pad {
    padding: 10px 16px 0;
  }
  .foot {
    padding-bottom: 10px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
    margin-top: 8px;
  }
  th,
  td {
    padding: 7px 12px;
    text-align: left;
    border-bottom: 0.5px solid var(--hairline);
    white-space: nowrap;
  }
  thead th {
    color: var(--ink-2);
    font-weight: 500;
    font-size: 12px;
    white-space: normal;
    vertical-align: bottom;
  }
  tbody tr:last-child td {
    border-bottom: 0;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .sort {
    border: 0;
    background: transparent;
    padding: 0;
    font: inherit;
    color: inherit;
    text-align: inherit;
    cursor: pointer;
  }
  .sort:hover {
    color: var(--ink);
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .state i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--s1);
  }
  .state.full i {
    background: var(--critical);
  }
  .state.partial i {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--s1);
  }
  .state.running i {
    opacity: 0.55;
  }
  @container main (max-width: 1040px) {
    th,
    td {
      padding-left: 8px;
      padding-right: 8px;
    }
    .opt {
      display: none;
    }
  }
</style>
