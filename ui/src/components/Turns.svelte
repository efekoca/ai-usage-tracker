<script lang="ts">
  import { app, latest, toolColor } from '../lib/store.svelte'
  import { api, type TurnCost, type TurnPart, type Turns } from '../lib/api'
  import { agentLabel, clientLabel, fmtCompact, fmtDateTime, fmtInt, fmtMoney, fmtPct, fmtSpan, has, t, toolLabel } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import Segmented from './Segmented.svelte'

  const FIRST = 5
  const STEP = 20

  let data = $state<Turns | null>(null)
  let sort = $state<'cost' | 'newest'>('cost')
  let shown = $state(FIRST)
  let open = $state<string | null>(null)
  $effect(() => {
    void app.tick
    return latest(
      () => api.turnCosts($state.snapshot(app.period), $state.snapshot(app.filter)),
      (d) => (data = d),
      () => (data = null),
    )
  })
  $effect(() => {
    void sort
    shown = FIRST
  })

  const rows = $derived.by(() => {
    const list = [...(data?.turns ?? [])]
    return sort === 'cost' ? list.sort((a, b) => b.cost_usd - a.cost_usd || b.started_ms - a.started_ms) : list.sort((a, b) => b.started_ms - a.started_ms)
  })

  const summary = $derived.by(() => {
    const costs = (data?.turns ?? []).map((x) => x.cost_usd).sort((a, b) => b - a)
    const total = costs.reduce((a, b) => a + b, 0)
    const top = costs.slice(0, Math.max(1, Math.ceil(costs.length / 10))).reduce((a, b) => a + b, 0)
    return {
      n: costs.length,
      // the lower of the two middle ones, as the archive's own medians are taken
      median: costs.length ? costs[Math.floor(costs.length / 2)] : 0,
      topShare: total > 0 && costs.length >= 10 ? (top / total) * 100 : null,
    }
  })

  const project = (x: TurnCost) => (x.hidden ? `${t('projects.hidden')}${x.project_id === null ? '' : ` #${x.project_id}`}` : x.project || t('projects.noProject'))
  const origin = (x: TurnCost) => (x.origin === 'human' ? '' : t(has(`turns.origin.${x.origin}`) ? `turns.origin.${x.origin}` : 'turns.origin.other'))
  const priced = (x: TurnCost) => x.unpriced_requests < x.requests || x.cost_usd > 0
  const agentsCost = (x: TurnCost) => x.parts.filter((p) => p.agent !== null).reduce((a, p) => a + p.cost_usd, 0)
  /** How much of the turn's cost its subagents made, 0 to 100. */
  const agentShare = (x: TurnCost) => (x.cost_usd > 0 ? Math.min(100, (agentsCost(x) / x.cost_usd) * 100) : 0)
  const partName = (p: TurnPart) => (p.agent === null ? agentLabel(null) : p.threads > 1 ? `${agentLabel(p.agent)} × ${fmtInt(p.threads)}` : agentLabel(p.agent))

  function openSession(x: TurnCost) {
    if (x.session_id === null) return
    app.openSession = x.tool + x.session_id
    app.view = 'sessions'
  }
</script>

{#if data && (data.turns.length > 0 || data.tracked)}
  <section class="card table-card">
    <div class="head">
      <div class="title">
        <h2>{t('turns.title')}</h2>
        <p class="subtle small help">{t('turns.help')}</p>
      </div>
      {#if data.turns.length > 1}
        <Segmented
          label={t('turns.sort')}
          bind:value={sort}
          options={[
            { value: 'cost', label: t('turns.sort.cost') },
            { value: 'newest', label: t('turns.sort.newest') },
          ]}
        />
      {/if}
    </div>
    {#if data.turns.length === 0}
      <p class="muted pad">{t('turns.none')}</p>
    {:else}
      <dl class="facts">
        <div><dt>{t('turns.count')}</dt><dd class="num">{fmtInt(summary.n)}</dd></div>
        <div title={t('turns.median.help')}><dt>{t('turns.median')}</dt><dd class="num">{fmtMoney(summary.median)}</dd></div>
        <div title={t('turns.top.help')}><dt>{t('turns.top')}</dt><dd class="num">{summary.topShare === null ? '—' : fmtPct(summary.topShare)}</dd></div>
      </dl>
      <div class="scroll">
        <table>
          <thead>
            <tr>
              <th scope="col" class="toggle-col"><span class="sr-only">{t('sessions.detail')}</span></th>
              <th scope="col">{t('turns.turn')}</th>
              <th scope="col" class="num">{t('sessions.requests')}</th>
              <th scope="col" class="num">{t('turns.agents')}</th>
              <th scope="col" class="num">{t('turns.calls')}</th>
              <th scope="col" class="num wide">{t('sessions.duration')}</th>
              <th scope="col" class="num">{t('metric.apiEq')}</th>
            </tr>
          </thead>
          <tbody>
            {#each rows.slice(0, shown) as x (x.id)}
              {@const isOpen = open === x.id}
              <tr class:open={isOpen}>
                <td class="toggle-col">
                  <button class="btn ghost icon" aria-expanded={isOpen} aria-label="{t('sessions.detail')}: {fmtDateTime(x.started_ms)}" onclick={() => (open = isOpen ? null : x.id)}>
                    <span class="chev" class:down={isOpen}><Icon name="chevron" size={13} /></span>
                  </button>
                </td>
                <th scope="row">
                  <span class="when"><i class="dot" style:background={toolColor[x.tool]} title={toolLabel(x.tool)}></i>{fmtDateTime(x.started_ms)}</span>
                  <span class="where subtle small">
                    <span class="project" title={project(x)}>{project(x)}</span>
                    <span aria-hidden="true">·</span>
                    <span>{t('turns.nth', { n: fmtInt(x.ordinal) })}</span>
                    {#if origin(x)}<span class="origin">{origin(x)}</span>{/if}
                  </span>
                </th>
                <td class="num">{fmtInt(x.requests)}</td>
                <td class="num">{x.agents ? fmtInt(x.agents) : '—'}</td>
                <td class="num">
                  {x.tool_calls ? fmtInt(x.tool_calls) : '—'}
                  {#if x.tool_failed}<span class="failed small">{t('turns.failed', { n: fmtInt(x.tool_failed) })}</span>{/if}
                </td>
                <td class="num wide">{fmtSpan(x.ended_ms - x.started_ms)}</td>
                <td class="num cost">
                  <b>{priced(x) ? fmtMoney(x.cost_usd) : '—'}</b>
                  {#if x.agents && x.cost_usd > 0}
                    <span class="split" role="img" aria-label={t('turns.split', { main: fmtMoney(x.cost_usd - agentsCost(x)), agents: fmtMoney(agentsCost(x)) })} title={t('turns.split', { main: fmtMoney(x.cost_usd - agentsCost(x)), agents: fmtMoney(agentsCost(x)) })}>
                      <i style:width="{agentShare(x)}%"></i>
                    </span>
                  {/if}
                </td>
              </tr>
              {#if isOpen}
                <tr class="detail">
                  <td></td>
                  <td colspan="6">
                    <div class="panel">
                      <ul class="parts">
                        {#each x.parts as p (p.agent)}
                          <li>
                            <span class="part" class:agent={p.agent !== null}><i></i>{partName(p)}</span>
                            <span class="subtle num">{t('sessions.requestCount', { n: fmtInt(p.requests) })}</span>
                            <b class="num">{priced(x) ? fmtMoney(p.cost_usd) : '—'}</b>
                          </li>
                        {/each}
                      </ul>
                      <div class="meta">
                        <span class="models">
                          {#each x.models.slice(0, 3) as m (m)}<span class="chip" title={m}>{m}</span>{/each}
                          {#if x.models.length > 3}<span class="subtle small">+{x.models.length - 3}</span>{/if}
                        </span>
                        <span>{t('turns.tokens', { tokens: fmtCompact(x.total_tokens) })}</span>
                        <span>{toolLabel(x.tool)}{x.client ? ` · ${clientLabel(`${x.tool}:${x.client}`, x.client)}` : ''}</span>
                        <span class="spacer"></span>
                        {#if x.session_id !== null}
                          <button class="btn ghost small" onclick={() => openSession(x)}>{t('turns.openSession')}</button>
                        {/if}
                      </div>
                      {#if x.unpriced_requests > 0}
                        <p class="subtle small unpriced">{t('turns.unpriced', { n: fmtInt(x.unpriced_requests) })}</p>
                      {/if}
                    </div>
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
      {#if rows.length > shown}
        <div class="more"><button class="btn" onclick={() => (shown += STEP)}>{t('sessions.more', { n: fmtInt(rows.length - shown) })}</button></div>
      {/if}
    {/if}
    <p class="subtle small pad note">
      {t('turns.note')}
      {#if data.requests_outside > 0}{t('turns.outside', { n: fmtInt(data.requests_outside) })}{/if}
    </p>
  </section>
{/if}

<style>
  .table-card {
    padding: 6px 0;
    margin-bottom: 16px;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px 20px;
    padding: 12px 20px 10px;
    flex-wrap: wrap;
  }
  .title {
    flex: 1 1 380px;
    min-width: 0;
  }
  h2 {
    margin-bottom: 2px;
  }
  .help {
    margin: 0;
    max-width: 720px;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 32px;
    margin: 0;
    padding: 2px 20px 12px;
  }
  .facts div {
    display: grid;
    gap: 2px;
  }
  .facts dt {
    font-size: 11.5px;
    color: var(--ink-3);
  }
  .facts dd {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    text-align: left;
  }
  .scroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 8px 10px;
    border-bottom: 0.5px solid var(--hairline);
    text-align: left;
    vertical-align: middle;
  }
  thead th {
    font-weight: 500;
    color: var(--ink-2);
    font-size: 12px;
    white-space: nowrap;
    border-top: 0.5px solid var(--hairline);
  }
  th:last-child,
  td:last-child {
    padding-right: 20px;
  }
  tbody th {
    font-weight: 500;
    min-width: 190px;
  }
  .when,
  .where {
    display: flex;
    align-items: center;
    white-space: nowrap;
  }
  .where {
    gap: 5px;
    padding-left: 16px;
    font-weight: 400;
  }
  .project {
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .origin {
    padding: 0 6px;
    border-radius: 5px;
    background: var(--surface-press);
    color: var(--ink-2);
  }
  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-right: 8px;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  /* on a narrow window the cost stays in view without scrolling sideways */
  @container main (max-width: 760px) {
    .wide {
      display: none;
    }
    tbody th {
      min-width: 150px;
    }
    .project {
      max-width: 110px;
    }
  }
  .failed {
    display: block;
    color: var(--bad-ink);
  }
  .cost b {
    font-weight: 600;
  }
  /* the whole bar is the turn's cost; the filled end is what its subagents made of it */
  .split {
    display: block;
    width: 64px;
    height: 3px;
    margin: 4px 0 0 auto;
    border-radius: 2px;
    background: var(--s1);
    overflow: hidden;
  }
  .split i {
    display: block;
    height: 100%;
    margin-left: auto;
    background: var(--s7);
  }
  .toggle-col {
    width: 34px;
    padding-left: 13px;
    padding-right: 0;
  }
  .icon {
    width: 26px;
    height: 26px;
    padding: 0;
    justify-content: center;
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }
  .chev.down {
    transform: rotate(90deg);
  }
  tr.open td,
  tr.open th {
    border-bottom-color: transparent;
  }
  .detail td {
    background: var(--surface-hover, transparent);
    padding-top: 14px;
    padding-bottom: 14px;
  }
  .panel {
    border: 0.5px solid var(--hairline-strong);
    border-radius: 12px;
    overflow: hidden;
    max-width: 720px;
  }
  .parts {
    list-style: none;
    margin: 0;
    padding: 4px 0;
  }
  .parts li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(72px, auto);
    align-items: center;
    gap: 16px;
    padding: 6px 14px;
  }
  .part {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .part i {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--s1);
  }
  .part.agent i {
    background: var(--s7);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    padding: 6px 8px 6px 14px;
    border-top: 0.5px solid var(--hairline);
    font-size: 12.5px;
    color: var(--ink-2);
  }
  .models {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .chip {
    font-size: 11.5px;
    padding: 1px 7px;
    border-radius: 6px;
    background: var(--surface-press);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
  }
  .unpriced {
    margin: 0;
    padding: 0 14px 10px;
  }
  .spacer {
    flex: 1;
  }
  .more {
    display: flex;
    justify-content: center;
    padding: 12px;
  }
  .pad {
    padding: 0 20px;
  }
  .note {
    padding-top: 10px;
    padding-bottom: 14px;
    margin: 0;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
