<script lang="ts">
  import { app, latest, saveSettings } from '../lib/store.svelte'
  import { api, type ExperimentSide, type MarkerView, type ProjectRow } from '../lib/api'
  import { fmtCompact, fmtDate, fmtInt, fmtMoney, fmtPct, localDate, t, toolLabel } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import Select from './Select.svelte'

  const DAY = 86_400_000
  /** As many as the app keeps; one more would push the oldest out. */
  const MOST = 50
  /** Fewer than this on a side says too little to call a change better or worse. */
  const ENOUGH = { sessions: 3, calls: 20 }

  let rows = $state<MarkerView[]>([])
  let projects = $state<ProjectRow[]>([])
  let label = $state('')
  let date = $state(localDate())
  let project = $state('')
  let tool = $state('')
  let days = $state('7')

  $effect(() => {
    void [app.tick, app.settings?.markers.length]
    return latest(
      () => api.experiments(),
      (d) => (rows = d),
      () => (rows = []),
    )
  })
  $effect(() => {
    void app.tick
    api.projects().then((p) => (projects = p)).catch(() => {})
  })

  const tools = $derived([...new Set((app.report?.by_tool ?? []).map((g) => g.key))])
  const projectName = (p: { id: number; name: string; hidden: boolean }) => (p.hidden || !p.name ? `${t('projects.hidden')} #${p.id}` : p.name)

  const dayStart = $derived(new Date(`${date}T00:00:00`).getTime())
  // some engines open the calendar on a click by themselves, the others when asked
  function openPicker(e: MouseEvent) {
    try {
      ;(e.currentTarget as HTMLInputElement).showPicker?.()
    } catch {
      /* already open, or not allowed here: the field still takes the keyboard */
    }
  }
  const dayLabel = $derived(date === localDate() ? t('experiment.today') : Number.isFinite(dayStart) ? fmtDate(dayStart, 'short') : t('experiment.day'))
  const full = $derived((app.settings?.markers.length ?? 0) >= MOST)
  let adding = $state(false)
  // a day that has begun, with a few words for it, while there is room for one more
  const ready = $derived(!!label.trim() && Number.isFinite(dayStart) && dayStart > 0 && dayStart <= Date.now() && !full && !adding)

  async function add() {
    if (!ready) return
    // a second Enter or click while this one is saved must not add it twice
    adding = true
    try {
      const project_id = project ? Number(project) : null
      // the folder comes from the unmasked list; a hidden project's is not shown anywhere
      const project_path = project_id === null ? null : ((await api.projectsForSettings().catch(() => [])).find((p) => p.id === project_id)?.path ?? null)
      const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
      const marker = { id, at_ms: dayStart, label: label.trim(), project_id, project_path, tool: tool || null, days: Number(days) }
      await saveSettings((c) => ({ markers: [...c.markers, marker] }))
      label = ''
    } finally {
      adding = false
    }
  }
  const remove = (id: string) => saveSettings((c) => ({ markers: c.markers.filter((m) => m.id !== id) }))

  type Metric = { key: string; before: string; after: string; change: string; tone: '' | 'good' | 'bad'; bars: [number, number] | null }
  /** One figure before and after: `lower` says a smaller one is the better; `points` that it is a percentage. */
  function metric(key: string, b: number | null, a: number | null, show: (n: number) => string, enough: boolean, lower: boolean, points: boolean): Metric {
    const out: Metric = { key, before: b === null ? '—' : show(b), after: a === null ? '—' : show(a), change: '', tone: '', bars: null }
    if (b === null || a === null) return out
    // each bar as a share of the larger of the two
    const most = Math.max(b, a)
    out.bars = most > 0 ? [(b / most) * 100, (a / most) * 100] : [0, 0]
    const moved = points ? a - b : b === 0 ? 0 : ((a - b) / b) * 100
    const size = Math.abs(moved)
    if ((points ? size < 0.05 : size < 0.5) || (!points && b === 0)) return out
    const sign = moved > 0 ? '+' : '−'
    out.change = points ? t('experiment.points', { n: `${sign}${fmtPct(size, 1).replace('%', '').trim()}` }) : `${sign}${fmtPct(size, size < 10 ? 1 : 0)}`
    // a change of less than five percent, or one point, is not called either way
    if (enough && size >= (points ? 1 : 5)) out.tone = moved < 0 === lower ? 'good' : 'bad'
    return out
  }
  function metrics(m: MarkerView): Metric[] {
    const [b, a] = [m.before, m.after]
    // with too few sessions on a side nothing is called better or worse, whatever the figure
    const sides = (pick: (s: ExperimentSide) => number, min: number) => !thin(m) && pick(b) >= min && pick(a) >= min
    return [
      // requests without a price are in neither cost; a few change little, many make the two unlike
      metric('cost', b.cost_per_session, a.cost_per_session, (n) => fmtMoney(n), sides((s) => s.sessions, ENOUGH.sessions) && !muchUnpriced(m), true, false),
      metric('start', b.starting_load, a.starting_load, (n) => fmtCompact(n), sides((s) => s.started, ENOUGH.sessions), true, false),
      metric('errors', b.tool_error_pct, a.tool_error_pct, (n) => fmtPct(n, 1), sides((s) => s.tool_calls, ENOUGH.calls), true, true),
      metric('cache', b.cache_hit_pct, a.cache_hit_pct, (n) => fmtPct(n, 1), sides((s) => s.sessions, ENOUGH.sessions), false, true),
    ]
  }
  const waiting = (m: MarkerView) => m.after.to_ms <= m.after.from_ms
  const passed = (m: MarkerView) => Math.max(0, Math.min(m.days, Math.floor((m.after.to_ms - m.after.from_ms) / DAY)))
  const scope = (m: MarkerView) =>
    [m.project_id === null ? '' : m.project_gone ? t('experiment.projectGone') : projectName({ id: m.project_id, name: m.project, hidden: m.project_hidden }), m.tool ? toolLabel(m.tool) : ''].filter(Boolean)
  /** `days` calendar days from the day of `at`: a day the clock changes on is still one day. */
  function addDays(at: number, days: number): number {
    const d = new Date(at)
    d.setDate(d.getDate() + days)
    return d.getTime()
  }
  // the days on each side of a marker, named by their first and last
  const beforeDays = (at: number, days: number) => `${fmtDate(addDays(at, -days), 'short')} – ${fmtDate(addDays(at, -1), 'short')}`
  const afterDays = (at: number, days: number) => `${fmtDate(at, 'short')} – ${fmtDate(addDays(at, days - 1), 'short')}`
  const used = (s: ExperimentSide) => t('experiment.used', { sessions: fmtInt(s.sessions), requests: fmtInt(s.requests) })
  const thin = (m: MarkerView) => m.before.sessions < ENOUGH.sessions || m.after.sessions < ENOUGH.sessions
  const unpriced = (m: MarkerView) => m.before.unpriced_requests + m.after.unpriced_requests
  /** More of a side's requests than this without a price, and the two costs are not weighed against each other. */
  const UNPRICED = 0.05
  const muchUnpriced = (m: MarkerView) => [m.before, m.after].some((s) => s.requests > 0 && s.unpriced_requests / s.requests > UNPRICED)
</script>

<h2 class="section">{t('experiment.title')}</h2>
<p class="subtle small lead">{t('experiment.lead')}</p>

<form
  class="add"
  onsubmit={(e) => {
    e.preventDefault()
    add()
  }}
>
  <div class="command">
    <svg class="flag" aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M5 21V4M5 4h11l-2 4 2 4H5" /></svg>
    <input class="what-changed" bind:value={label} maxlength="80" aria-label={t('experiment.label')} placeholder={t('experiment.label.example')} />
    <div class="chips">
      <!-- the system's own date field lies unseen over the button: a click or a key reaches it in every engine -->
      <label class="chip day">
        <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="5" width="17" height="15.5" rx="2.5" /><path d="M3.5 10h17M8 3v4M16 3v4" /></svg>
        <span>{dayLabel}</span>
        <input class="picker" type="date" bind:value={date} max={localDate()} aria-label={t('experiment.day')} onclick={openPicker} />
      </label>
      <Select label={t('common.project')} value={project} options={[{ value: '', label: t('experiment.allProjects') }, ...projects.map((p) => ({ value: String(p.id), label: projectName(p) }))]} onchange={(v) => (project = v)} />
      <Select label={t('experiment.tool')} value={tool} options={[{ value: '', label: t('experiment.allTools') }, ...tools.map((k) => ({ value: k, label: toolLabel(k) }))]} onchange={(v) => (tool = v)} />
      <Select label={t('experiment.days')} value={days} options={['3', '7', '14', '30'].map((n) => ({ value: n, label: t('experiment.around', { n }) }))} onchange={(v) => (days = v)} />
      <button class="go" type="submit" disabled={!ready} aria-label={t('experiment.add')} title={t('experiment.add')}><Icon name="plus" size={16} /></button>
    </div>
  </div>
  <p class="subtle small under">
    {#if full}
      {t('experiment.full', { n: MOST })}
    {:else if Number.isFinite(dayStart)}
      {t('experiment.compares', { before: beforeDays(dayStart, Number(days)), after: afterDays(dayStart, Number(days)) })} · {t('experiment.enter')}
    {/if}
  </p>
</form>

{#each rows as m (m.id)}
  <article class="card marker">
    <div class="head">
      <div class="what">
        <h3>{m.label}</h3>
        <span class="subtle small">
          {[fmtDate(m.at_ms, 'long'), ...scope(m), t('experiment.span', { n: m.days }), !waiting(m) && passed(m) < m.days ? t('experiment.running', { done: passed(m), days: m.days }) : ''].filter(Boolean).join(' · ')}
        </span>
      </div>
      <span class="legend subtle small" aria-hidden="true"><span class="swatch"></span>{t('experiment.before')}<span class="swatch after"></span>{t('experiment.after')}</span>
      <button class="btn ghost icon" aria-label="{t('common.remove')}: {m.label}" title={t('common.remove')} onclick={() => remove(m.id)}><Icon name="trash" size={15} /></button>
    </div>
    <div class="table" role="table" aria-label={m.label}>
      {#each metrics(m) as x (x.key)}
        <div class="row" role="row">
          <span class="name" role="rowheader">{t(`experiment.metric.${x.key}`)}</span>
          <span class="before num subtle" role="cell" aria-label="{t('experiment.before')}: {x.before}">{x.before}</span>
          <span class="bars" aria-hidden="true">
            {#if x.bars}
              <span class="bar" style="width: {x.bars[0]}%"></span>
              <span class="bar after" style="width: {x.bars[1]}%"></span>
            {/if}
          </span>
          <b class="after num" role="cell" aria-label="{t('experiment.after')}: {x.after}">{x.after}</b>
          <span class="delta" role="cell">{#if x.change}<span class="change num {x.tone}">{x.change}</span>{/if}</span>
        </div>
      {/each}
    </div>
    <p class="subtle small basis">
      {beforeDays(m.at_ms, m.days)}: {used(m.before)} · {afterDays(m.at_ms, m.days)}: {waiting(m) ? t('experiment.notYet') : used(m.after)}
    </p>
    {#if !waiting(m) && thin(m)}<p class="subtle small thin"><Icon name="info" size={13} />{t('experiment.thin')}</p>{/if}
    {#if muchUnpriced(m)}<p class="subtle small thin"><Icon name="info" size={13} />{t('experiment.unpriced', { n: fmtInt(unpriced(m)) })}</p>{/if}
  </article>
{:else}
  <div class="none">
    <span class="tile" aria-hidden="true"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M5 21V4M5 4h11l-2 4 2 4H5" /></svg></span>
    <b>{t('experiment.none')}</b>
    <p class="subtle small">{t('experiment.none.hint')}</p>
  </div>
{/each}

<style>
  .section {
    margin: 24px 0 4px;
  }
  .lead {
    margin: 0 0 12px;
    max-width: 760px;
  }
  .tile {
    flex: none;
    width: 38px;
    height: 38px;
    border-radius: 11px;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: color-mix(in srgb, var(--accent) 62%, var(--ink));
  }
  .add {
    display: grid;
    gap: 7px;
    margin-bottom: 14px;
  }
  /* one line: what changed, with the day and what it is narrowed to as small buttons in it */
  .command {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    min-height: 54px;
    box-sizing: border-box;
    padding: 9px 9px 9px 16px;
    border-radius: 15px;
    background: var(--surface);
    border: 0.5px solid var(--hairline-strong);
    box-shadow: var(--shadow);
    transition: border-color var(--dur) var(--ease);
  }
  .command:focus-within {
    border-color: var(--accent);
  }
  .flag {
    flex: none;
    color: color-mix(in srgb, var(--accent) 62%, var(--ink));
  }
  /* as wide as its example at least: when that no longer fits beside the buttons, they go below */
  .what-changed {
    flex: 1 1 380px;
    min-width: 0;
    text-overflow: ellipsis;
    height: 36px;
    padding: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--ink);
    font: inherit;
    font-size: 14.5px;
  }
  .what-changed::placeholder {
    color: var(--ink-3);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .chip,
  .chips :global(.trigger) {
    height: 32px;
    border-radius: 9px;
    border: 0.5px solid var(--hairline);
    background: var(--surface-2);
    color: var(--ink-2);
    font-size: 12.5px;
    font-weight: 500;
    box-shadow: none;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    white-space: nowrap;
  }
  .chip:hover {
    border-color: var(--axis);
  }
  .day {
    position: relative;
    cursor: pointer;
  }
  .day:focus-within {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .picker {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    padding: 0;
    border: 0;
    opacity: 0;
    cursor: pointer;
  }
  .go {
    flex: none;
    width: 36px;
    height: 36px;
    margin-left: 2px;
    display: grid;
    place-items: center;
    border-radius: 11px;
    border: 0;
    background: var(--accent);
    color: var(--accent-ink);
  }
  .go:disabled {
    opacity: 0.45;
  }
  .under {
    margin: 0;
    padding-left: 16px;
    min-height: 1.45em;
  }
  .marker {
    display: grid;
    gap: 10px;
    margin-bottom: 12px;
    padding: 18px 20px 14px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .what {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 3px;
  }
  h3 {
    margin: 0;
    font-size: 15.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
    overflow-wrap: anywhere;
  }
  .legend {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--ink) 28%, transparent);
  }
  .swatch.after {
    margin-left: 8px;
    background: var(--accent);
  }
  .row {
    display: grid;
    grid-template-columns: minmax(150px, 190px) 72px minmax(0, 1fr) 72px 100px;
    align-items: center;
    gap: 14px;
    padding: 11px 0;
    border-top: 0.5px solid var(--hairline);
  }
  .name {
    font-weight: 500;
  }
  .before {
    text-align: right;
  }
  .bars {
    display: grid;
    gap: 3px;
  }
  .bar {
    height: 6px;
    min-width: 2px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--ink) 28%, transparent);
  }
  .bar.after {
    background: var(--accent);
  }
  b.after {
    font-size: 14px;
    font-weight: 600;
  }
  .delta {
    justify-self: end;
  }
  .change {
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--ink-2);
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }
  .change.good {
    background: color-mix(in srgb, var(--good-ink) 14%, transparent);
    color: var(--good-ink);
  }
  .change.bad {
    background: color-mix(in srgb, var(--bad-ink) 14%, transparent);
    color: var(--bad-ink);
  }
  .basis {
    margin: 0;
    padding-top: 10px;
    border-top: 0.5px solid var(--hairline);
  }
  .thin {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
  }
  .none {
    display: grid;
    justify-items: center;
    gap: 8px;
    padding: 28px 24px;
    border: 1px dashed var(--hairline-strong);
    border-radius: var(--radius);
    text-align: center;
  }
  .none b {
    font-size: 15px;
    font-weight: 600;
  }
  .none p {
    margin: 0;
    max-width: 520px;
  }
  @container main (max-width: 860px) {
    .legend {
      display: none;
    }
    .row {
      grid-template-columns: minmax(0, 1fr) auto auto;
      row-gap: 7px;
    }
    .name {
      grid-column: 1 / -1;
    }
    .before {
      text-align: left;
    }
    .bars {
      grid-column: 1 / -1;
      grid-row: 3;
    }
  }
</style>
