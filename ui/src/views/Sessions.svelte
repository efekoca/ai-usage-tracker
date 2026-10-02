<script lang="ts">
  // Sessions in the selected period: summary tiles and a sortable, searchable table whose rows
  // open into a per-model breakdown. Follows the toolbar's period and filters.
  import { app, toolColor } from '../lib/store.svelte'
  import { api, type SessionRow, type Sessions } from '../lib/api'
  import { clientLabel, fmtCompact, fmtDateTime, fmtInt, fmtSpan, fmtMoney, fmtPct, t, toolLabel } from '../lib/i18n.svelte'
  import StatTile from '../components/StatTile.svelte'
  import Icon from '../components/Icon.svelte'

  let data = $state<Sessions | null>(null)
  let failed = $state('')
  $effect(() => {
    void app.tick
    api
      .sessions($state.snapshot(app.period), $state.snapshot(app.filter))
      .then((d) => {
        data = d
        failed = ''
      })
      .catch((e) => (failed = String(e)))
  })

  type Key = 'started' | 'duration' | 'requests' | 'tokens' | 'cost' | 'hit' | 'context'
  let sortKey = $state<Key>("started")
  let desc = $state(true)
  let query = $state('')
  let shown = $state(50)
  let open = $state<string | null>(null)
  let copied = $state<string | null>(null)

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
    const q = query.trim().toLocaleLowerCase()
    const list = (data?.sessions ?? []).filter(
      (s) =>
        !q ||
        s.session_id.toLowerCase().includes(q) ||
        (!s.hidden && s.project.toLocaleLowerCase().includes(q)) ||
        s.models.some((m) => m.model.toLowerCase().includes(q)),
    )
    const f = value[sortKey]
    return list.sort((a, b) => (desc ? f(b) - f(a) : f(a) - f(b)) || b.started_ms - a.started_ms)
  })
  $effect(() => {
    void query
    void sortKey
    shown = 50
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
      <div class="scroll">
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
                  <button class="btn ghost icon" aria-expanded={isOpen} aria-label={t('sessions.detail')} onclick={() => (open = isOpen ? null : s.tool + s.session_id)}>
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
                      <div>
                        <h3>{t('sessions.modelsUsed')}</h3>
                        <ul>
                          {#each s.models as m (m.model)}
                            <li>
                              <span class="chip">{m.model}</span>
                              <span class="subtle small">{fmtInt(m.events)} {t('sessions.requests').toLocaleLowerCase()}</span>
                              <span class="spacer"></span>
                              <span class="num small">{fmtCompact(m.total_tokens)}</span>
                              <span class="num small money">{m.unpriced ? '—' : fmtMoney(m.cost_usd)}</span>
                            </li>
                          {/each}
                        </ul>
                      </div>
                      <dl>
                        <dt>{t('metric.input')}</dt><dd class="num">{fmtCompact(s.totals.tokens.input)}</dd>
                        <dt>{t('metric.cacheRead')}</dt><dd class="num">{fmtCompact(s.totals.tokens.cache_read)}</dd>
                        <dt>{t('metric.cacheWrite')}</dt><dd class="num">{fmtCompact(s.totals.tokens.cache_write)}</dd>
                        <dt>{t('metric.output')}</dt><dd class="num">{fmtCompact(s.totals.tokens.output)}</dd>
                        <dt>{t('sessions.avgContext')}</dt><dd class="num">{fmtCompact(s.avg_context)}</dd>
                        <dt>{t('sessions.tool')}</dt><dd>{toolLabel(s.tool)}{s.client ? ` · ${clientLabel(`${s.tool}:${s.client}`, s.client)}` : ''}</dd>
                        <dt>{t('sessions.id')}</dt>
                        <dd class="id">
                          <code>{s.session_id}</code>
                          <button class="btn ghost small" onclick={() => copy(s.session_id)}>{copied === s.session_id ? t('sessions.copied') : t('sessions.copy')}</button>
                        </dd>
                      </dl>
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
  @media (max-width: 900px) {
    .tiles {
      grid-template-columns: 1fr 1fr;
    }
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 18px 10px;
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
    padding-left: 12px;
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
    padding-top: 4px;
    padding-bottom: 16px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1.2fr 1fr;
    gap: 24px;
  }
  @media (max-width: 900px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
  h3 {
    font-size: 12px;
    font-weight: 600;
    color: var(--ink-2);
    margin: 0 0 6px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
  }
  .money {
    min-width: 64px;
    text-align: right;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 16px;
    margin: 0;
  }
  dt {
    color: var(--ink-2);
  }
  dd {
    margin: 0;
  }
  dd.num {
    text-align: left;
  }
  .id {
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }
  .id code {
    font-size: 11.5px;
    word-break: break-all;
  }
  .more {
    display: flex;
    justify-content: center;
    padding: 12px;
  }
  .pad {
    padding: 0 18px;
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
