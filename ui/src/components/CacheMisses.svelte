<script lang="ts">
  import { api, type CacheMisses, type MissEvent, type MissProject } from '../lib/api'
  import { app, latest } from '../lib/store.svelte'
  import { fmtCompact, fmtDate, fmtDateTime, fmtInt, fmtPct, has, localDate, t } from '../lib/i18n.svelte'
  import { shortModel } from '../lib/caches'
  import StatTile from './StatTile.svelte'
  import BarValue from './BarValue.svelte'
  import Icon from './Icon.svelte'
  import Columns from './charts/Columns.svelte'

  let data = $state<CacheMisses | null>(null)
  let failed = $state('')
  $effect(() => {
    void app.tick
    return latest(
      () => api.cacheMisses($state.snapshot(app.period), $state.snapshot(app.filter)),
      (d) => {
        data = d
        failed = ''
      },
      (e) => {
        data = null
        failed = String(e)
      },
    )
  })

  const shown = $derived(!!app.settings?.enabled_sources.includes('claude_code'))
  const bridge = $derived(!!app.settings?.capture.statusline)
  const any = $derived(!!data && (data.misses > 0 || data.rebuilds > 0))

  const UNKNOWN = 'undetermined'
  // Claude Code may name a cause this version does not know yet: it is shown as written
  const label = (cause: string) =>
    cause === UNKNOWN ? t('cachemiss.undetermined') : has(`cachemiss.cause.${cause}`) ? t(`cachemiss.cause.${cause}`) : cause.replaceAll('_', ' ').replace(/^./, (c) => c.toUpperCase())
  const help = (cause: string) => (has(`cachemiss.cause.${cause}.help`) ? t(`cachemiss.cause.${cause}.help`) : undefined)
  // a cause keeps its color whatever else the period holds
  const KNOWN: Record<string, string> = { ttl_expired_5m: 'var(--s4)', tools_changed: 'var(--s1)', system_prompt_changed: 'var(--s7)', likely_server_side: 'var(--s5)', [UNKNOWN]: 'var(--axis)' }
  const SPARE = ['var(--s3)', 'var(--s2)', 'var(--s6)', 'var(--s8)']
  const colorOf = $derived.by(() => {
    const other = (data?.causes ?? []).map((c) => c.cause).filter((c) => !KNOWN[c])
    return (cause: string) => KNOWN[cause] ?? SPARE[Math.max(0, other.indexOf(cause)) % SPARE.length]
  })

  /** Misses Claude Code gave no cause for count as one more slice. */
  const slices = $derived(data ? [...data.causes, ...(data.undetermined ? [{ cause: UNKNOWN, misses: data.undetermined }] : [])] : [])

  /** Misses day by day, by cause, from the first day with one to the last. */
  const daily = $derived.by(() => {
    if (!data?.days.length) return null
    const by = new Map(data.days.map((d) => [d.date, Object.fromEntries([...d.causes.map((c) => [c.cause, c.misses]), ...(d.undetermined ? [[UNKNOWN, d.undetermined]] : [])])]))
    const at = (date: string) => new Date(`${date}T12:00:00`)
    const dates: string[] = []
    for (const d = at(data.days[0].date), end = at(data.days.at(-1)!.date); d <= end; d.setDate(d.getDate() + 1)) dates.push(localDate(d))
    const kept = dates.slice(-92)
    return { dates: kept, values: kept.map((d) => by.get(d) ?? {}), series: slices.map((s) => ({ key: s.cause, label: label(s.cause), color: colorOf(s.cause) })) }
  })

  const keyOf = (p: MissProject) => (p.project_id === null ? 'none' : String(p.project_id))
  const name = (p: MissProject) => (p.hidden ? `${t('projects.hidden')}${p.project_id !== null ? ` #${p.project_id}` : ''}` : p.project || t('projects.noProject'))
  /** Its main cause, the misses Claude Code could not explain, or a rebuild alone. */
  const mainCause = (p: MissProject) => p.cause ?? (p.misses ? UNKNOWN : null)
  const most = $derived(Math.max(1, ...(data?.projects ?? []).map((p) => p.misses)))
  let opened = $state<string | null>(null)

  /** "Tool definitions changed", "2 misses · ...", or a rebuild Claude Code expected. */
  function what(e: MissEvent): string {
    if (!e.misses) return t('cachemiss.rebuild')
    const why = e.causes.length ? e.causes.map(label).join(', ') : t('cachemiss.undetermined')
    return e.misses > 1 ? `${t('cachemiss.count', { n: fmtInt(e.misses) })} · ${why}` : why
  }
  function openSession(e: MissEvent) {
    app.openSession = 'claude_code' + e.session_id
    app.view = 'sessions'
  }
</script>

{#if shown}
  {#if failed}
    <section class="card"><p class="muted">{failed}</p></section>
  {:else if data}
    <section class="card" aria-labelledby="cache-misses">
      <header>
        <h2 id="cache-misses">{t('cachemiss.title')}</h2>
        <p class="subtle small lead">{t('cachemiss.lead')}</p>
      </header>

      {#if !data.reported}
        <div class="setup">
          <p class="muted">{bridge ? t('cachemiss.waiting') : t('cachemiss.setup')}</p>
          {#if !bridge}<button class="btn" onclick={() => (app.view = 'sources')}>{t('settings.cacheTimer.statusline.open')}</button>{/if}
        </div>
      {:else if !any}
        <p class="muted empty">{t('cachemiss.none')}</p>
      {:else}
        <div class="tiles">
          <StatTile label={t('cachemiss.misses')} hint={t('cachemiss.misses.help')} value={fmtInt(data.misses)}>
            <span class="subtle">{t('cachemiss.in', { n: fmtInt(data.sessions) })}</span>
          </StatTile>
          <StatTile label={t('cachemiss.recached')} value={fmtCompact(data.recached_tokens)} />
          <StatTile label={t('cachemiss.rebuilds')} hint={t('cachemiss.rebuilds.help')} value={fmtInt(data.rebuilds)} />
        </div>

        {#if data.misses > 0}
          <div class="split">
            <div class="part">
              <h3>{t('cachemiss.causes')}</h3>
              <div class="mixbar" role="img" aria-label={slices.map((s) => `${label(s.cause)} ${fmtInt(s.misses)}`).join(', ')}>
                {#each slices as s (s.cause)}<span style:flex={s.misses} style:background={colorOf(s.cause)} title="{label(s.cause)}: {fmtInt(s.misses)}"></span>{/each}
              </div>
              <table class="legend">
                <tbody>
                  {#each slices as s (s.cause)}
                    <tr>
                      <td><i class="swatch" style:background={colorOf(s.cause)}></i></td>
                      <td class="name">
                        <span class="cname">{label(s.cause)}</span>
                        {#if help(s.cause)}
                          <span class="tip">
                            <button class="tip-btn" aria-label={t('cachemiss.about', { cause: label(s.cause) })} aria-describedby="miss-tip-{s.cause}"><Icon name="info" size={12} /></button>
                            <span class="tip-box" role="tooltip" id="miss-tip-{s.cause}">{help(s.cause)}</span>
                          </span>
                        {/if}
                      </td>
                      <td class="num">{fmtInt(s.misses)}</td>
                      <td class="num subtle">{fmtPct((s.misses / data.misses) * 100, 0)}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
              {#if slices.length > 1}<p class="subtle small note">{t('cachemiss.causes.note')}</p>{/if}
            </div>
            {#if daily}
              <div class="part">
                <h3>{t('cachemiss.byDay')}</h3>
                <Columns dates={daily.dates} values={daily.values} series={daily.series} format={fmtInt} height={190} legend={false} ariaLabel={t('cachemiss.byDay')} xLabel={(k) => fmtDate(k, 'short')} />
              </div>
            {/if}
          </div>
        {/if}

        {#if data.projects.length}
          <h3 class="projects-title">{t('cachemiss.byProject')}</h3>
          <div class="scroll">
            <table class="projects">
              <thead>
                <tr>
                  <th scope="col" class="toggle-col"><span class="sr-only">{t('sessions.detail')}</span></th>
                  <th scope="col">{t('cachemiss.col.project')}</th>
                  <th scope="col">{t('cachemiss.misses')}</th>
                  <th scope="col">{t('cachemiss.col.mainCause')}</th>
                  <th scope="col" class="num">{t('cachemiss.col.recached')}</th>
                  <th scope="col" class="num">{t('cachemiss.col.last')}</th>
                </tr>
              </thead>
              <tbody>
                {#each data.projects as g (keyOf(g))}
                  {@const isOpen = opened === keyOf(g)}
                  {@const cause = mainCause(g)}
                  <!-- the whole row opens with a click; the arrow button carries it for the keyboard and screen readers -->
                  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
                  <tr class="group" class:open={isOpen} onclick={() => (opened = isOpen ? null : keyOf(g))}>
                    <td class="toggle-col">
                      <button class="btn ghost icon" aria-expanded={isOpen} aria-label="{t('sessions.detail')}: {name(g)}">
                        <span class="chev" class:down={isOpen}><Icon name="chevron" size={13} /></span>
                      </button>
                    </td>
                    <td class="conv"><span class="pname" class:dim={g.hidden || !g.project}>{name(g)}</span></td>
                    <td class="count">{#if g.misses}<BarValue pct={(g.misses / most) * 100} text={fmtInt(g.misses)} />{:else}<span class="subtle">—</span>{/if}</td>
                    <td class="why" class:soft={!cause}>
                      {#if cause}<i class="dot" style:background={colorOf(cause)}></i>{label(cause)}{:else}{t('cachemiss.rebuild')}{/if}
                    </td>
                    <td class="num">{g.recached_tokens ? fmtCompact(g.recached_tokens) : '—'}</td>
                    <td class="num when">{fmtDateTime(g.last_ms)}</td>
                  </tr>
                  {#if isOpen}
                    <tr class="detail">
                      <td></td>
                      <td colspan="5">
                        <ul class="events">
                          {#each g.events as e}
                            <li class:rebuild={!e.misses}>
                              <i class="dot" class:hollow={!e.misses} style:background={e.misses ? colorOf(e.causes[0] ?? UNKNOWN) : undefined}></i>
                              <span class="at">{fmtDateTime(e.ts_ms)}</span>
                              <span class="where">
                                {#if e.model}<span class="model">{shortModel(e.model)}</span>{/if}
                                {#if e.branch}<span class="chip" title={e.branch}><Icon name="branch" size={11} /><span class="branch">{e.branch}</span></span>{/if}
                              </span>
                              <span class="what" title={e.misses ? undefined : t('cachemiss.rebuilds.help')}>{what(e)}</span>
                              <span class="tok">{e.recached_tokens ? fmtCompact(e.recached_tokens) : '—'}</span>
                              <button class="btn ghost icon" aria-label="{t('cachemiss.open')}: {name(g)}, {fmtDateTime(e.ts_ms)}" title={t('cachemiss.open')} onclick={() => openSession(e)}><Icon name="external" size={13} /></button>
                            </li>
                          {/each}
                          {#if g.more}<li class="older">{t('cachemiss.older', { n: fmtInt(g.more) })}</li>{/if}
                        </ul>
                      </td>
                    </tr>
                  {/if}
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      {/if}
    </section>
  {/if}
{/if}

<style>
  section {
    margin-bottom: 16px;
  }
  h2 {
    margin: 0;
  }
  h3 {
    margin: 0 0 12px;
    font-size: 13px;
    font-weight: 600;
    color: var(--ink-2);
  }
  header {
    margin-bottom: 18px;
  }
  .lead {
    margin: 6px 0 0;
    max-width: 760px;
    line-height: 1.5;
  }
  .empty {
    margin: 0;
  }
  .setup {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 12px 16px;
  }
  .setup p {
    margin: 0;
    max-width: 640px;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 24px;
    padding-top: 16px;
    border-top: 0.5px solid var(--hairline);
  }
  .split {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr);
    gap: 32px;
    margin-top: 20px;
    padding-top: 18px;
    border-top: 0.5px solid var(--hairline);
  }
  @container main (max-width: 860px) {
    .split {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  @container main (max-width: 520px) {
    .tiles {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .part {
    min-width: 0;
  }
  .mixbar {
    display: flex;
    gap: 2px;
    height: 14px;
    border-radius: 5px;
    overflow: hidden;
    margin: 4px 0 12px;
  }
  .mixbar span {
    min-width: 0;
  }
  .legend {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .legend td {
    padding: 6px 0;
    white-space: nowrap;
  }
  .legend td:first-child {
    width: 18px;
  }
  .legend .swatch {
    display: inline-block;
    vertical-align: -1px;
  }
  .legend .name {
    color: var(--ink-2);
    cursor: default;
  }
  .tip {
    position: relative;
    display: inline-flex;
    margin-left: 4px;
    vertical-align: -2px;
  }
  .tip-btn {
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--ink-3);
    cursor: help;
  }
  .tip-btn:hover,
  .tip-btn:focus-visible {
    color: var(--ink);
    background: var(--surface-hover);
  }
  /* beside the icon, so it covers neither the bar above nor the rows below */
  .tip-box {
    position: absolute;
    left: calc(100% + 8px);
    top: 50%;
    z-index: 5;
    width: max-content;
    max-width: 260px;
    padding: 8px 10px;
    border-radius: 9px;
    border: 0.5px solid var(--hairline-strong);
    background: var(--surface);
    box-shadow: var(--shadow-pop);
    color: var(--ink-2);
    font-size: 12px;
    line-height: 1.45;
    white-space: normal;
    opacity: 0;
    transform: translate(-4px, -50%);
    transition:
      opacity var(--dur) var(--ease),
      transform var(--dur) var(--ease);
    pointer-events: none;
  }
  .tip:hover .tip-box,
  .tip-btn:focus-visible + .tip-box {
    opacity: 1;
    transform: translate(0, -50%);
  }
  .legend td.num {
    width: 1%;
    padding-left: 18px;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .note {
    margin: 8px 0 0;
  }
  .projects-title {
    margin-top: 22px;
    padding-top: 18px;
    border-top: 0.5px solid var(--hairline);
  }
  .scroll {
    overflow-x: auto;
  }
  .projects {
    width: 100%;
    min-width: 640px;
    border-collapse: separate;
    border-spacing: 0;
    font-size: 13px;
  }
  .projects th,
  .projects td {
    padding: 9px 10px;
    border-bottom: 0.5px solid var(--hairline);
    text-align: left;
    vertical-align: middle;
  }
  .projects th {
    padding-top: 0;
    font-weight: 500;
    color: var(--ink-2);
    font-size: 12px;
    white-space: nowrap;
  }
  .projects th:last-child,
  .projects td:last-child {
    padding-right: 0;
  }
  .projects th.num,
  .projects td.num {
    text-align: right;
  }
  .toggle-col {
    width: 26px;
    padding-left: 4px !important;
    padding-right: 4px !important;
  }
  .icon {
    width: 26px;
    height: 26px;
    padding: 0;
    justify-content: center;
    color: var(--ink-3);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease);
  }
  .chev.down {
    transform: rotate(90deg);
  }
  /* one rounded band across the row, the arrow inside it and not a box of its own */
  .group {
    cursor: pointer;
  }
  .group td {
    transition: background var(--dur) var(--ease);
  }
  .group:hover td {
    background: var(--surface-hover);
  }
  .group:hover td:first-child {
    border-radius: 8px 0 0 8px;
  }
  .group:hover td:last-child {
    border-radius: 0 8px 8px 0;
  }
  .group .icon:hover {
    background: transparent;
  }
  .group:hover .icon {
    color: var(--ink);
  }
  tr.open td {
    border-bottom-color: transparent;
  }
  /* the project takes the room the rest leaves, and a long name shortens */
  .conv {
    width: 34%;
    max-width: 0;
  }
  .pname {
    display: block;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pname.dim {
    color: var(--ink-2);
    font-weight: 500;
  }
  .count {
    width: 150px;
  }
  .why {
    white-space: nowrap;
  }
  .why.soft {
    color: var(--ink-3);
  }
  .dot {
    display: inline-block;
    flex: none;
    width: 8px;
    height: 8px;
    margin-right: 8px;
    border-radius: 50%;
    vertical-align: 1px;
  }
  .dot.hollow {
    box-sizing: border-box;
    border: 1.5px solid var(--ink-3);
    background: transparent;
  }
  .when {
    color: var(--ink-2);
  }
  .detail > td {
    padding-top: 0;
    padding-bottom: 14px;
  }
  .events {
    list-style: none;
    margin: 0;
    padding: 4px 0;
    border: 0.5px solid var(--hairline);
    border-radius: 10px;
    background: var(--surface-hover);
  }
  .events li {
    display: grid;
    grid-template-columns: 8px 112px minmax(0, 1.1fr) minmax(0, 1.3fr) 64px 26px;
    align-items: center;
    gap: 12px;
    padding: 6px 8px 6px 14px;
    font-size: 12.5px;
  }
  .events li + li {
    border-top: 0.5px solid var(--hairline);
  }
  .events .dot {
    margin: 0;
  }
  .at {
    color: var(--ink-2);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .where {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
  }
  .model {
    flex: none;
    color: var(--ink-2);
  }
  /* a long branch name shortens and shows whole on hover */
  .chip {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 100%;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 7px;
    border-radius: 6px;
    background: var(--surface-2);
    font-size: 11.5px;
    color: var(--ink-2);
  }
  .chip :global(svg) {
    flex: none;
  }
  .branch {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .what {
    min-width: 0;
  }
  .rebuild .what {
    color: var(--ink-3);
  }
  .tok {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .events li.older {
    display: block;
    padding-left: 34px;
    color: var(--ink-3);
  }
</style>
