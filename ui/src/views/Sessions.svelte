<script lang="ts">
  import { app, latest, toolColor } from '../lib/store.svelte'
  import { api, type SessionRow, type Sessions } from '../lib/api'
  import { clientLabel, fmtCompact, fmtDateTime, fmtInt, fmtSpan, fmtMoney, fmtPct, lower, searchKey, t, toolLabel } from '../lib/i18n.svelte'
  import StatTile from '../components/StatTile.svelte'
  import Icon from '../components/Icon.svelte'
  import SessionTimeline from '../components/SessionTimeline.svelte'

  let data = $state<Sessions | null>(null)
  let failed = $state('')
  $effect(() => {
    void app.tick
    return latest(
      () => api.sessions($state.snapshot(app.period), $state.snapshot(app.filter)),
      (d) => {
        data = d
        failed = ''
      },
      (e) => (failed = String(e)),
    )
  })

  type Key = 'started' | 'duration' | 'requests' | 'tokens' | 'cost' | 'hit' | 'context'
  let sortKey = $state<Key>("started")
  let desc = $state(true)
  let query = $state('')
  let shown = $state(50)
  let open = $state<string | null>(null)
  let copied = $state<string | null>(null)
  let visibleW = $state(0)

  const prompt = (s: SessionRow) => s.totals.tokens.input + s.totals.tokens.cache_read + s.totals.tokens.cache_write
  const hit = (s: SessionRow) => (prompt(s) > 0 ? s.totals.tokens.cache_read / prompt(s) : 0)
  const dur = (s: SessionRow) => s.ended_ms - s.started_ms
  const value: Record<Key, (s: SessionRow) => number> = {
    started: (s) => s.started_ms,
    duration: dur,
    requests: (s) => s.totals.events,
    tokens: (s) => s.totals.total_tokens,
    cost: (s) => s.totals.cost_usd,
    hit,
    context: (s) => s.max_context,
  }
  const projectName = (s: SessionRow) => (s.hidden ? `${t('projects.hidden')}${s.project_id !== null ? ` #${s.project_id}` : ''}` : s.project || t('projects.noProject'))

  const rows = $derived.by(() => {
    const q = searchKey(query.trim())
    const list = (data?.sessions ?? []).filter(
      (s) =>
        !q ||
        searchKey(s.session_id).includes(q) ||
        (!s.hidden && searchKey(s.project).includes(q)) ||
        s.models.some((m) => searchKey(m.model).includes(q)),
    )
    const f = value[sortKey]
    return list.sort((a, b) => (desc ? f(b) - f(a) : f(a) - f(b)) || b.started_ms - a.started_ms)
  })
  $effect(() => {
    void query
    void sortKey
    shown = 50
  })
  $effect(() => {
    const key = app.openSession
    if (!key || !data) return
    const s = data.sessions.find((x) => x.tool + x.session_id === key)
    if (s) {
      query = s.session_id
      open = key
    }
    app.openSession = null
  })

  const summary = $derived.by(() => {
    const list = data?.sessions ?? []
    const n = list.length
    const cost = list.reduce((a, s) => a + s.totals.cost_usd, 0)
    const costliest = list.reduce<SessionRow | null>((a, s) => (!a || s.totals.cost_usd > a.totals.cost_usd ? s : a), null)
    const durations = list.map(dur).sort((a, b) => a - b)
    return {
      n,
      avgDuration: n ? durations.reduce((a, b) => a + b, 0) / n : 0,
      avgCost: n ? cost / n : 0,
      costliest,
      maxContext: list.reduce((a, s) => Math.max(a, s.max_context), 0),
    }
  })

  function sortBy(k: Key) {
    if (sortKey === k) desc = !desc
    else {
      sortKey = k
      desc = true
    }
  }
  async function copy(id: string) {
    try {
      await navigator.clipboard.writeText(id)
      copied = id
      setTimeout(() => (copied = null), 1500)
    } catch {
      /* clipboard unavailable */
    }
  }
  const cols: [Key | null, string, boolean][] = [
    ['started', 'sessions.started', false],
    [null, 'sessions.project', false],
    [null, 'sessions.models', false],
    ['duration', 'sessions.duration', true],
    ['requests', 'sessions.requests', true],
    ['tokens', 'metric.tokens', true],
    ['cost', 'metric.apiEq', true],
    ['hit', 'sessions.hit', true],
    ['context', 'sessions.maxContext', true],
  ]
</script>

{#if failed}
  <div class="banner"><Icon name="warning" size={16} />{t('common.error', { e: failed })}</div>
{:else if !data}
  <p class="muted">{t('common.loading')}</p>
{:else}
  <section class="tiles card">
    <StatTile hero label={t('sessions.count')} value={fmtInt(summary.n)} />
    <StatTile label={t('sessions.avgDuration')} hint={t('sessions.durationHelp')} value={summary.n ? fmtSpan(summary.avgDuration) : '—'} />
    <StatTile label={t('sessions.avgCost')} hint={t('metric.apiEq.help')} value={summary.n ? fmtMoney(summary.avgCost) : '—'} />
    <StatTile label={t('sessions.maxContext')} value={summary.n ? fmtCompact(summary.maxContext) : '—'}>
      {#if summary.costliest}
        <span class="subtle">{t('sessions.costliest')}: {fmtMoney(summary.costliest.totals.cost_usd)}</span>
      {/if}
    </StatTile>
  </section>

  <section class="card table-card">
    <div class="head">
      <h2>{t('sessions.list')}</h2>
      <span class="spacer"></span>
      <input class="field search" type="search" placeholder={t('sessions.search')} aria-label={t('sessions.search')} bind:value={query} />
    </div>
    {#if data.sessions.length === 0}
      <p class="muted pad">{t('sessions.none')}</p>
    {:else if rows.length === 0}
      <p class="muted pad">{t('sessions.noMatch')}</p>
    {:else}
      <div class="scroll" bind:clientWidth={visibleW} style:--visible="{visibleW}px">
        <table>
          <thead>
            <tr>
              <th scope="col" class="toggle-col"><span class="sr-only">{t('sessions.detail')}</span></th>
              {#each cols as [k, label, num] (label)}
                <th scope="col" class:num aria-sort={k && sortKey === k ? (desc ? 'descending' : 'ascending') : undefined}>
                  {#if k}
                    <button class="sort" class:active={sortKey === k} onclick={() => sortBy(k)}>
                      {t(label)}{#if sortKey === k}<span aria-hidden="true">{desc ? '↓' : '↑'}</span>{/if}
                    </button>
                  {:else}{t(label)}{/if}
                </th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each rows.slice(0, shown) as s (s.tool + s.session_id)}
              {@const isOpen = open === s.tool + s.session_id}
              <tr class:open={isOpen}>
                <td class="toggle-col">
                  <button class="btn ghost icon" aria-expanded={isOpen} aria-label="{t('sessions.detail')}: {fmtDateTime(s.started_ms)}" onclick={() => (open = isOpen ? null : s.tool + s.session_id)}>
                    <span class="chev" class:down={isOpen}><Icon name="chevron" size={13} /></span>
                  </button>
                </td>
                <td class="nowrap">
                  <i class="dot" style="background:{toolColor[s.tool]}" title={toolLabel(s.tool)}></i>{fmtDateTime(s.started_ms)}
                </td>
                <td class="proj" title={projectName(s)}>{projectName(s)}</td>
                <td class="models">
                  {#each s.models.slice(0, 2) as m (m.model)}<span class="chip">{m.model}</span>{/each}
                  {#if s.models.length > 2}<span class="subtle small">+{s.models.length - 2}</span>{/if}
                </td>
                <td class="num">{fmtSpan(dur(s))}</td>
                <td class="num">{fmtInt(s.totals.events)}</td>
                <td class="num">{fmtCompact(s.totals.total_tokens)}</td>
                <td class="num">{s.totals.unpriced_events === s.totals.events ? '—' : fmtMoney(s.totals.cost_usd)}</td>
                <td class="num">{prompt(s) > 0 ? fmtPct(hit(s) * 100, 1) : '—'}</td>
                <td class="num">{s.max_context ? fmtCompact(s.max_context) : '—'}</td>
              </tr>
              {#if isOpen}
                <tr class="detail">
                  <td></td>
                  <td colspan={cols.length}>
                    <div class="grid">
                      <div class="panel">
                        <div class="stats">
                          {#each [
                            [t('metric.input'), fmtCompact(s.totals.tokens.input), 'var(--ink-3)'],
                            [t('metric.cacheRead'), fmtCompact(s.totals.tokens.cache_read), 'var(--s1)'],
                            [t('metric.cacheWrite'), fmtCompact(s.totals.tokens.cache_write), 'var(--warning)'],
                            [t('metric.output'), fmtCompact(s.totals.tokens.output), 'var(--s7)'],
                            [t('sessions.avgContext'), fmtCompact(s.avg_context), null],
                            [t('sessions.hit'), prompt(s) > 0 ? fmtPct(hit(s) * 100, 1) : '—', null],
                          ] as [label, v, color] (label)}
                            <div class="stat">
                              <span class="label">{label}</span>
                              <span class="value num">{v}</span>
                              {#if color}<i class="tick" style:background={color}></i>{/if}
                            </div>
                          {/each}
                        </div>
                        <div class="by-model">
                          <span class="label">{t('sessions.models')}</span>
                          <div class="used">
                            {#each s.models as m (m.model)}
                              <span class="model">
                                <span class="chip" title={m.model}>{m.model}</span>
                                <span class="subtle">{fmtInt(m.events)} {lower(t('sessions.requests'))}</span>
                                <span class="num">{fmtCompact(m.total_tokens)}</span>
                                <b class="num">{m.unpriced ? '—' : fmtMoney(m.cost_usd)}</b>
                              </span>
                            {/each}
                          </div>
                        </div>
                        <div class="meta">
                          <span class="tool" title={t('sessions.tool')}>
                            <i class="dot" style:background={toolColor[s.tool]}></i><span class="ink">{toolLabel(s.tool)}</span>
                            {#if s.client}<span aria-hidden="true">·</span><span>{clientLabel(`${s.tool}:${s.client}`, s.client)}</span>{/if}
                          </span>
                          <span class="id">
                            <code title={t('sessions.id')}>{s.session_id}</code>
                            <button class="btn ghost icon" aria-label={copied === s.session_id ? t('sessions.copied') : t('sessions.copy')} title={copied === s.session_id ? t('sessions.copied') : t('sessions.copy')} onclick={() => copy(s.session_id)}>
                              <Icon name={copied === s.session_id ? 'check' : 'copy'} size={14} />
                            </button>
                          </span>
                        </div>
                      </div>
                      <SessionTimeline session={s} />
                    </div>
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
      {#if rows.length > shown}
        <div class="more"><button class="btn" onclick={() => (shown += 50)}>{t('sessions.more', { n: fmtInt(rows.length - shown) })}</button></div>
      {/if}
    {/if}
    <p class="subtle small pad note">
      {t('sessions.partial')}
      {#if data.events_without_session > 0}{t('sessions.withoutId', { n: fmtInt(data.events_without_session) })}{/if}
    </p>
  </section>
{/if}

<style>
  .tiles {
    display: grid;
    grid-template-columns: 1.4fr 1fr 1fr 1fr;
    gap: 8px;
    margin-bottom: 16px;
  }
  @container main (max-width: 840px) {
    .tiles {
      grid-template-columns: 1fr 1fr;
    }
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 20px 10px;
    flex-wrap: wrap;
  }
  .search {
    width: min(320px, 100%);
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
  th {
    font-weight: 500;
    color: var(--ink-2);
    font-size: 12px;
    white-space: nowrap;
  }
  /* the last cells and the expand arrow line up with the card's heading */
  th:last-child,
  td:last-child {
    padding-right: 20px;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .sort {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    color: inherit;
    display: inline-flex;
    gap: 4px;
  }
  .sort.active {
    color: var(--ink);
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
  .nowrap {
    white-space: nowrap;
  }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-right: 8px;
    vertical-align: 1px;
  }
  .proj {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .models {
    white-space: nowrap;
  }
  .chip {
    display: inline-block;
    font-size: 11.5px;
    padding: 1px 7px;
    border-radius: 6px;
    background: var(--surface-press);
    margin-right: 4px;
    white-space: nowrap;
  }
  tr.open td {
    border-bottom-color: transparent;
  }
  .detail td {
    background: var(--surface-hover, transparent);
    padding-top: 14px;
    padding-bottom: 16px;
  }
  /* a wide table scrolls sideways; the details keep to what is in view */
  .grid {
    position: sticky;
    left: 0;
    max-width: calc(var(--visible, 100%) - 34px - 42px);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .panel {
    container: panel / inline-size;
    border: 0.5px solid var(--hairline-strong);
    border-radius: 12px;
    overflow: hidden;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    overflow: hidden;
    border-bottom: 0.5px solid var(--hairline);
  }
  /* a Turkish label would wrap and push its value below the others */
  @container panel (max-width: 800px) {
    .stats {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 14px;
    min-width: 0;
    /* the outer edges fall outside .stats, so only the lines between cells show */
    margin: 0 -0.5px -0.5px 0;
    border-right: 0.5px solid var(--hairline);
    border-bottom: 0.5px solid var(--hairline);
  }
  .label {
    font-size: 11.5px;
    color: var(--ink-3);
  }
  .value {
    font-size: 18px;
    font-weight: 600;
    text-align: left;
  }
  .tick {
    width: 22px;
    height: 3px;
    border-radius: 2px;
  }
  .by-model {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 14px;
  }
  .by-model .label {
    font-size: 12px;
    font-weight: 600;
    color: var(--ink-2);
    min-width: 64px;
    line-height: 26px;
  }
  .used {
    flex: 1;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
  }
  .model {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    padding: 3px 8px 3px 3px;
    border-radius: 8px;
    border: 0.5px solid var(--hairline);
    background: var(--surface-2);
    white-space: nowrap;
    max-width: 100%;
    min-width: 0;
    box-sizing: border-box;
  }
  /* a long name gives way, so its requests, tokens and cost stay in view */
  .model .chip {
    margin-right: 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .model > :not(.chip) {
    flex: none;
  }
  .model .subtle {
    font-size: 12px;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 6px 16px;
    padding: 6px 8px 6px 14px;
    border-top: 0.5px solid var(--hairline);
    font-size: 12.5px;
    color: var(--ink-2);
  }
  .tool {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .tool .dot {
    margin-right: 2px;
    vertical-align: 0;
  }
  .ink {
    color: var(--ink);
  }
  .id {
    display: flex;
    gap: 4px;
    align-items: center;
    min-width: 0;
  }
  .id code {
    font-size: 11.5px;
    color: var(--ink-3);
    overflow-wrap: anywhere;
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
  .spacer {
    flex: 1;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
