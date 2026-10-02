<script lang="ts">
  import { app, toolColor } from '../lib/store.svelte'
  import { fmtCompact, fmtDate, fmtInt, fmtMoney, t, toolLabel } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import CalendarHeatmap from '../components/charts/CalendarHeatmap.svelte'

  let mode: 'calendar' | 'table' = $state('calendar')
  let metric: 'tokens' | 'cost' = $state(app.settings?.primary_metric ?? 'tokens')
  let selected: string | null = $state(null)

  const r = $derived(app.report)
  const days = $derived(r?.daily ?? [])
  const fmt = $derived(metric === 'tokens' ? fmtCompact : (v: number) => fmtMoney(v))
  const val = (i: number) => (metric === 'tokens' ? days[i].tokens : days[i].cost_usd)
  const tools = $derived([...new Set(days.flatMap((d) => Object.keys(d.by_tool)))].sort())
  const sel = $derived(days.find((d) => d.date === selected) ?? null)
  const rows = $derived([...days].reverse())
</script>

<div class="head">
  <Segmented label={t('daily.title')} options={[{ value: 'calendar', label: t('daily.calendar') }, { value: 'table', label: t('daily.table') }]} bind:value={mode} />
  <Segmented label={t('metric.tokens')} options={[{ value: 'tokens', label: t('metric.tokens') }, { value: 'cost', label: t('metric.cost') }]} bind:value={metric} />
</div>

{#if !r}
  <p class="muted">{t('common.loading')}</p>
{:else if mode === 'calendar'}
  <section class="card">
    <CalendarHeatmap {days} value={val} format={fmt} ariaLabel={t('daily.calendar')} {selected} onselect={(d) => (selected = d)} />
  </section>
  {#if sel}
    <section class="card day">
      <h2>{fmtDate(sel.date, 'long')}</h2>
      {#if sel.events === 0}
        <p class="muted">{t('daily.noData')}</p>
      {:else}
        <div class="facts">
          <div><span class="muted small">{t('metric.tokens')}</span><b class="big">{fmtCompact(sel.tokens)}</b></div>
          <div><span class="muted small">{t('metric.apiEq')}</span><b class="big">{fmtMoney(sel.cost_usd)}</b></div>
          <div><span class="muted small">{t('metric.events')}</span><b class="big">{fmtInt(sel.events)}</b></div>
        </div>
        <ul class="tools">
          {#each Object.entries(sel.by_tool) as [tool, tok] (tool)}
            <li><i class="swatch" style="background:{toolColor[tool]}"></i>{toolLabel(tool)}<span class="spacer"></span><span class="num">{fmtCompact(tok)}</span><span class="num muted">{fmtMoney(sel.cost_by_tool[tool] ?? 0)}</span></li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
{:else}
  <section class="card table-card">
    <table>
      <thead>
        <tr>
          <th scope="col">{t('daily.date')}</th>
          {#each tools as tl (tl)}<th scope="col" class="num">{toolLabel(tl)}</th>{/each}
          <th scope="col" class="num">{t('metric.total')}</th>
          <th scope="col" class="num">{t('metric.apiEq')}</th>
          <th scope="col" class="num">{t('metric.events')}</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as d (d.date)}
          <tr class:zero={d.events === 0}>
            <th scope="row">{fmtDate(d.date, 'medium')}</th>
            {#each tools as tl (tl)}<td class="num">{d.by_tool[tl] ? fmtCompact(d.by_tool[tl]) : '—'}</td>{/each}
            <td class="num">{fmtCompact(d.tokens)}</td>
            <td class="num">{fmtMoney(d.cost_usd)}</td>
            <td class="num">{fmtInt(d.events)}</td>
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
  }
  section {
    margin-bottom: 16px;
  }
  .day h2 {
    margin-bottom: 12px;
  }
  .facts {
    display: flex;
    gap: 40px;
    margin-bottom: 12px;
  }
  .facts div {
    display: flex;
    flex-direction: column;
  }
  .big {
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.02em;
  }
  .tools {
    list-style: none;
    padding: 0;
    margin: 0;
  }
  .tools li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 0;
    border-top: 0.5px solid var(--hairline);
  }
  .spacer {
    flex: 1;
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
