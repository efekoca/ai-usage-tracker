<script lang="ts">
  // Breakdown page: how much the subagents did (per tool, main conversation first) and which
  // tools the agents called, with the share of calls that returned an error.
  import { api, type AgentsTools, type Tool, type ToolRow } from '../lib/api'
  import { app, toolColor } from '../lib/store.svelte'
  import { fmtCompact, fmtInt, fmtMoney, fmtPct, has, t, toolLabel } from '../lib/i18n.svelte'
  import Segmented from './Segmented.svelte'
  import Icon from './Icon.svelte'

  let data = $state<AgentsTools | null>(null)
  $effect(() => {
    void app.tick
    api.agentsTools($state.snapshot(app.period), $state.snapshot(app.filter)).then((d) => (data = d)).catch(() => (data = null))
  })

  const agentLabel = (a: string | null) => (a === null ? t('agent.main') : has(`agent.${a}`) ? t(`agent.${a}`) : a)
  const tools = $derived([...new Set((data?.agents ?? []).map((a) => a.tool))] as Tool[])
  // per tool: share by cost, or by tokens when nothing in it has a price
  function shareOf(tool: Tool, cost: number, tokens: number) {
    const rows = (data?.agents ?? []).filter((a) => a.tool === tool)
    const c = rows.reduce((s, a) => s + a.totals.cost_usd, 0)
    if (c > 0) return cost / c
    const n = rows.reduce((s, a) => s + a.totals.total_tokens, 0)
    return n > 0 ? tokens / n : 0
  }
  // subagents' share of the tool's requests (always exact) and of its priced cost
  function subShare(tool: Tool): string {
    const rows = (data?.agents ?? []).filter((a) => a.tool === tool)
    const sub = rows.filter((a) => a.agent !== null)
    const req = sub.reduce((s, a) => s + a.totals.events, 0) / Math.max(1, rows.reduce((s, a) => s + a.totals.events, 0))
    const priced = sub.some((a) => a.totals.unpriced_events < a.totals.events)
    const total = rows.reduce((s, a) => s + a.totals.cost_usd, 0)
    const cost = sub.reduce((s, a) => s + a.totals.cost_usd, 0) / Math.max(1e-9, total)
    return priced && total > 0
      ? t('agents.subShare', { req: fmtPct(req * 100, 1), cost: fmtPct(cost * 100, 1) })
      : t('agents.subShareReq', { req: fmtPct(req * 100, 1) })
  }
  const hasSubagents = $derived((data?.agents ?? []).some((a) => a.agent !== null))

  let which: 'all' | Tool = $state('all')
  let showAll = $state(false)
  const toolTools = $derived([...new Set((data?.tools ?? []).map((x) => x.tool))] as Tool[])
  const rows = $derived((data?.tools ?? []).filter((x) => which === 'all' || x.tool === which))
  const totalCalls = $derived(rows.reduce((s, x) => s + x.calls, 0))
  const visible = $derived(showAll ? rows : rows.slice(0, 15))

  function toolName(r: ToolRow): { main: string; sub: string } {
    const mcp = r.name.match(/^mcp__(.+?)__(.+)$/)
    if (mcp) return { main: mcp[2], sub: t('tools.mcp', { server: mcp[1].replaceAll('_', ' ') }) }
    if (r.tool === 'codex' && has(`tools.codex.${r.name}`)) return { main: t(`tools.codex.${r.name}`), sub: r.name }
    return { main: r.name, sub: '' }
  }
</script>

{#if data}
  <div class="grid2">
    <section class="card table-card">
      <div class="pad">
        <h2>{t('agents.title')}</h2>
        <p class="subtle small lead">{t('agents.lead')}</p>
      </div>
      {#if !hasSubagents}
        <p class="muted pad empty">{t('agents.none')}</p>
      {/if}
      {#if data.agents.length}
        <table>
          <thead>
            <tr>
              <th scope="col">{t('agents.col.agent')}</th>
              <th scope="col" class="num">{t('agents.col.runs')}</th>
              <th scope="col" class="num">{t('agents.col.requests')}</th>
              <th scope="col" class="num">{t('metric.tokens')}</th>
              <th scope="col" class="num">{t('metric.apiEq')}</th>
              <th scope="col" class="num" title={t('agents.shareHelp')}>{t('agents.col.share')}</th>
            </tr>
          </thead>
          {#each tools as tool (tool)}
            <tbody>
              <tr class="group">
                <th scope="rowgroup" colspan="6">
                  <i class="dot" style="background:{toolColor[tool]}"></i>{toolLabel(tool)}
                  {#if data.agents.some((a) => a.tool === tool && a.agent !== null)}<span class="subtle small"> · {subShare(tool)}</span>{/if}
                </th>
              </tr>
              {#each data.agents.filter((a) => a.tool === tool) as a (a.agent ?? '')}
                {@const share = shareOf(tool, a.totals.cost_usd, a.totals.total_tokens)}
                <tr>
                  <th scope="row" class:main={a.agent === null}>{agentLabel(a.agent)}</th>
                  <td class="num">{a.agent === null ? '—' : fmtInt(a.runs)}</td>
                  <td class="num">{fmtInt(a.totals.events)}</td>
                  <td class="num">{fmtCompact(a.totals.total_tokens)}</td>
                  <td class="num">
                    {#if a.totals.unpriced_events === a.totals.events}<span class="unpriced" title={t('overview.unpricedAction')}><Icon name="warning" size={13} /> —</span>{:else}{fmtMoney(a.totals.cost_usd)}{/if}
                  </td>
                  {#if a.totals.unpriced_events === a.totals.events && a.totals.events > 0 && data.agents.some((x) => x.tool === tool && x.totals.cost_usd > 0)}
                    <td class="num subtle" title={t('overview.unpricedAction')}>—</td>
                  {:else}
                    <td class="num share"><span class="minibar" aria-hidden="true"><span style="width:{share * 100}%"></span></span>{fmtPct(share * 100, 1)}</td>
                  {/if}
                </tr>
              {/each}
            </tbody>
          {/each}
        </table>
      {/if}
    </section>

    <section class="card table-card">
      <div class="pad head">
        <div>
          <h2>{t('tools.title')}</h2>
          <p class="subtle small lead">{t('tools.lead')}</p>
        </div>
        {#if toolTools.length > 1}
          <Segmented label={t('tools.title')} bind:value={which} options={[{ value: 'all', label: t('tools.all') }, ...toolTools.map((x) => ({ value: x, label: toolLabel(x) }))]} />
        {/if}
      </div>
      {#if rows.length === 0}
        <p class="muted pad empty">{t('tools.none')}</p>
      {:else}
        <table>
          <thead>
            <tr>
              <th scope="col">{t('tools.col.tool')}</th>
              <th scope="col" class="num">{t('tools.col.calls')}</th>
              <th scope="col" class="num">{t('tools.col.share')}</th>
              <th scope="col" class="num" title={t('tools.errorsHelp')}>{t('tools.col.errors')}</th>
              <th scope="col" class="num">{t('tools.col.sub')}</th>
            </tr>
          </thead>
          <tbody>
            {#each visible as r (r.tool + r.name)}
              {@const n = toolName(r)}
              <tr>
                <th scope="row">
                  <span class="tname"><i class="dot" style="background:{toolColor[r.tool]}" title={toolLabel(r.tool)}></i><span>{n.main}</span></span>
                  {#if n.sub}<span class="subtle small sub">{n.sub}</span>{/if}
                </th>
                <td class="num">{fmtInt(r.calls)}</td>
                <td class="num share"><span class="minibar" aria-hidden="true"><span style="width:{(r.calls / Math.max(1, totalCalls)) * 100}%"></span></span>{fmtPct((r.calls / Math.max(1, totalCalls)) * 100, 1)}</td>
                <td class="num">
                  {#if r.known === 0}<span class="subtle small" title={t('tools.errorsHelp')}>{t('tools.noOutcome')}</span>{:else}{fmtPct((r.failed / r.known) * 100, 1)} <span class="subtle small">({fmtInt(r.failed)})</span>{/if}
                </td>
                <td class="num">{r.by_subagents ? fmtInt(r.by_subagents) : '—'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <div class="pad foot">
          {#if rows.length > 15}
            <button class="btn ghost small" onclick={() => (showAll = !showAll)}>{showAll ? t('tools.showLess') : t('tools.showAll', { n: rows.length })}</button>
          {/if}
          <span class="subtle small">{t('tools.total', { n: fmtInt(totalCalls) })}</span>
        </div>
        <p class="subtle small pad note">{t('tools.errorsHelp')} {toolTools.includes('codex') ? t('tools.codexNote') : ''} {data.filtered_by_session ? t('tools.bySession') : ''}</p>
      {/if}
    </section>
  </div>
{/if}

<style>
  .grid2 {
    display: grid;
    grid-template-columns: 1fr;
    gap: 16px;
    margin-bottom: 16px;
  }
  .table-card {
    padding: 6px 0;
    overflow-x: auto;
    min-width: 0;
  }
  .pad {
    padding: 10px 16px 0;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
    flex-wrap: wrap;
  }
  .lead {
    margin: 4px 0 8px;
    max-width: 560px;
  }
  .empty {
    padding-bottom: 12px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
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
  }
  tbody th {
    font-weight: 500;
  }
  tbody th.main {
    font-weight: 600;
  }
  tr.group th {
    font-size: 12px;
    color: var(--ink-2);
    font-weight: 600;
    background: var(--surface-2);
  }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    margin-right: 7px;
    vertical-align: 1px;
  }
  .tname {
    display: inline-flex;
    align-items: center;
  }
  .sub {
    margin-left: 6px;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .share {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }
  .minibar {
    width: 44px;
    height: 5px;
    border-radius: 3px;
    background: var(--surface-hover);
    overflow: hidden;
  }
  .minibar span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .unpriced {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    color: var(--ink-2);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 12px;
    padding-bottom: 4px;
  }
  .note {
    padding-bottom: 10px;
    white-space: normal;
    max-width: 640px;
  }
</style>
