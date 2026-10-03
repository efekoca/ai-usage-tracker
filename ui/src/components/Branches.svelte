<script lang="ts">
  // Projects page: usage per project and git branch.
  import { api, type Branches } from '../lib/api'
  import { app, setFilter, toolColor } from '../lib/store.svelte'
  import { fmtCompact, fmtDateTime, fmtInt, fmtMoney, fmtPct, t, toolLabel } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import BarValue from './BarValue.svelte'

  let data = $state<Branches | null>(null)
  $effect(() => {
    void app.tick
    api.branches($state.snapshot(app.period), $state.snapshot(app.filter)).then((d) => (data = d)).catch(() => (data = null))
  })
  const total = $derived(Math.max(1e-9, (data?.rows ?? []).reduce((s, r) => s + r.totals.cost_usd, 0)))
  const coverage = $derived(data ? (data.events_with_branch / Math.max(1, data.events_with_branch + data.events_without_branch)) * 100 : 0)
  let showAll = $state(false)
  const visible = $derived(showAll ? (data?.rows ?? []) : (data?.rows ?? []).slice(0, 20))
  const project = (r: Branches['rows'][number]) => (r.project_id === null ? t('projects.noProject') : r.hidden ? `${t('projects.hidden')} #${r.project_id}` : r.project)
  function focus(id: number | null) {
    if (id === null) return
    setFilter({ ...app.filter, projects: [id] })
  }
</script>

{#if data}
  <section class="card table-card">
    <div class="pad">
      <h2>{t('branches.title')}</h2>
      <p class="subtle small lead">{t('branches.lead')} {data.rows.length ? t('branches.coverage', { pct: fmtPct(coverage, 0) }) : ''}</p>
    </div>
    {#if data.rows.length === 0}
      <p class="muted pad empty">{t('branches.empty')}</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th scope="col">{t('branches.col.project')}</th>
            <th scope="col">{t('branches.col.branch')}</th>
            <th scope="col" class="opt">{t('branches.col.tools')}</th>
            <th scope="col" class="num">{t('branches.col.sessions')}</th>
            <th scope="col" class="num">{t('branches.col.requests')}</th>
            <th scope="col" class="num">{t('metric.tokens')}</th>
            <th scope="col" class="num">{t('metric.apiEq')}</th>
            <th scope="col" class="num">{t('breakdown.share')}</th>
            <th scope="col" class="num">{t('branches.col.last')}</th>
          </tr>
        </thead>
        <tbody>
          {#each visible as r, i ((r.project_id ?? -1) + ':' + (r.branch ?? (r.branch_hidden ? `#${i}` : '')))}
            <tr>
              <th scope="row">
                {#if r.project_id !== null}
                  <button class="link" onclick={() => focus(r.project_id)} title={t('branches.filter')}><Icon name="folder" size={14} />{project(r)}</button>
                {:else}
                  <span class="muted">{project(r)}</span>
                {/if}
              </th>
              <td>
                {#if r.branch}<code>{r.branch}</code>{:else if r.branch_hidden}<span class="subtle">{t('branches.hidden')}</span>{:else}<span class="subtle" title={t('branches.noneHelp')}>{t('branches.none')}</span>{/if}
              </td>
              <td class="opt">
                <span class="tools" aria-label={r.tools.map(toolLabel).join(', ')}>{#each r.tools as tool (tool)}<i class="dot" style="background:{toolColor[tool]}" title={toolLabel(tool)}></i>{/each}</span>
              </td>
              <td class="num">{fmtInt(r.sessions)}</td>
              <td class="num">{fmtInt(r.totals.events)}</td>
              <td class="num">{fmtCompact(r.totals.total_tokens)}</td>
              <td class="num">
                {#if r.totals.unpriced_events === r.totals.events}<span class="subtle" title={t('overview.unpricedAction')}>—</span>{:else}{fmtMoney(r.totals.cost_usd)}{/if}
              </td>
              <td class="num"><BarValue pct={(r.totals.cost_usd / total) * 100} text={fmtPct((r.totals.cost_usd / total) * 100, 1)} /></td>
              <td class="num subtle">{fmtDateTime(r.last_ms)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if data.rows.length > 20}
        <div class="pad foot"><button class="btn ghost small" onclick={() => (showAll = !showAll)}>{showAll ? t('tools.showLess') : t('tools.showAll', { n: data.rows.length })}</button></div>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .table-card {
    padding: 6px 0;
    overflow-x: auto;
    margin-top: 16px;
  }
  .pad {
    padding: 10px 16px 0;
  }
  .lead {
    margin: 4px 0 8px;
    max-width: 760px;
  }
  .empty,
  .foot {
    padding-bottom: 10px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 7px 10px;
    text-align: left;
    border-bottom: 0.5px solid var(--hairline);
    white-space: nowrap;
  }
  thead th {
    color: var(--ink-2);
    font-weight: 500;
    font-size: 12px;
  }
  tbody th {
    font-weight: 500;
  }
  tbody tr:last-child td,
  tbody tr:last-child th {
    border-bottom: 0;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 0;
    background: transparent;
    padding: 2px 6px;
    margin-left: -6px;
    border-radius: 6px;
    font: inherit;
    color: inherit;
  }
  .link:hover {
    background: var(--surface-hover);
  }
  .link :global(svg) {
    color: var(--ink-3);
  }
  code {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 12px;
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--surface-hover);
  }
  .tools {
    display: inline-flex;
    gap: 5px;
  }
  .dot {
    display: inline-block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  /* narrow windows: tighter cells, secondary columns hidden */
  @media (max-width: 1100px) {
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
