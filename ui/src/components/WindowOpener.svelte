<script lang="ts">
  import { app, saveSettings } from '../lib/store.svelte'
  import { api, type OpenerInfo, type OpenerRun, type Settings } from '../lib/api'
  import { clockCase, fmtCompact, fmtWhen, has, i18n, t, weekdayNames } from '../lib/i18n.svelte'
  import AutoCard from './AutoCard.svelte'

  const DAY = 24 * 60
  const WINDOW = 5 * 60

  let info = $state<OpenerInfo | null>(null)
  let busy = $state(false)
  let error = $state('')

  const o = $derived(app.settings?.window_opener)
  const load = () =>
    api
      .windowOpenerInfo()
      .then((i) => (info = i))
      .catch(() => {})
  $effect(() => {
    // the next run follows the time and the days
    void [app.tick, o?.enabled, o?.minute, o?.days.length]
    load()
  })

  const save = (patch: Partial<Settings['window_opener']>) => saveSettings((c) => ({ window_opener: { ...c.window_opener, ...patch } }))
  const hm = (minute: number) => `${String(Math.floor(minute / 60) % 24).padStart(2, '0')}:${String(minute % 60).padStart(2, '0')}`
  function setClock(value: string) {
    const [h, m] = value.split(':').map(Number)
    if (Number.isInteger(h) && Number.isInteger(m)) save({ minute: h * 60 + m })
  }
  // the last day stays on: a scheduler with no day would never run
  const setDay = (day: number, on: boolean) => o && (on || o.days.length > 1) && save({ days: on ? [...o.days, day].sort() : o.days.filter((d) => d !== day) })

  async function now() {
    busy = true
    error = ''
    try {
      info = await api.openWindowNow()
    } catch (e) {
      error = String(e)
    } finally {
      busy = false
    }
  }

  const pct = (minute: number) => (minute / DAY) * 100
  /** The parts of a window on today's line: one that runs past midnight goes on from the left. */
  function pieces(from: number, first: boolean) {
    return [
      [from, from + WINDOW],
      [from - DAY, from + WINDOW - DAY],
    ]
      .map(([a, b]) => [Math.max(0, a), Math.min(DAY, b)])
      .filter(([a, b]) => b > a)
      .map(([a, b]) => ({ left: pct(a), width: pct(b - a), first }))
  }
  const bands = $derived(o ? [...pieces(o.minute, true), ...pieces(o.minute + WINDOW, false)] : [])
  // the times under the line; the day's ends make room for one that sits near them
  const marks = $derived(
    o
      ? [
          { at: o.minute, label: t('opener.line.opens') },
          { at: (o.minute + WINDOW) % DAY, label: t('opener.line.resets') },
        ]
      : [],
  )
  const near = (minute: number) => marks.some((m) => Math.abs(m.at - minute) < 110)

  const why = (code: string) => (has(`opener.error.${code}`) ? t(`opener.error.${code}`) : code)
  const window = (r: OpenerRun) => (r.resets_ms === null ? '' : t(r.resets_ms > Date.now() ? 'opener.run.resets' : 'opener.run.reset', { at: clockCase(r.resets_ms, 'at') }))
  const tag = $derived(o?.enabled && info?.next_ms != null ? t('opener.next', { when: fmtWhen(info.next_ms) }) : '')
</script>

{#snippet icon()}
  <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="13" r="8" /><path d="M12 9v4l2.6 1.6M9 2h6" /></svg>
{/snippet}

{#if info && o}
  <AutoCard title={t('opener.title')} help={t('opener.help')} {tag} on={o.enabled} onchange={(v) => save({ enabled: v })} {icon}>
    {#if o.enabled}
      <div class="inset line" role="img" aria-label={t('opener.line', { opens: hm(o.minute), resets: hm(o.minute + WINDOW), ends: hm(o.minute + 2 * WINDOW) })}>
        <div class="workday"><span>{t('opener.line.workday')}</span></div>
        <div class="track">
          {#each bands as b (`${b.first}${b.left}`)}
            <div class="band" class:first={b.first} style="left: {b.left}%; width: {b.width}%">{#if b.width > 9}{t(b.first ? 'opener.line.first' : 'opener.line.second')}{/if}</div>
          {/each}
        </div>
        <div class="scale num">
          {#if !near(0)}<span class="edge">00:00</span>{/if}
          {#each marks as m (m.label)}
            <span class="mark" style="left: {pct(m.at)}%"><b>{hm(m.at)}</b><span>{m.label}</span></span>
          {/each}
          {#if !near(DAY)}<span class="edge end">24:00</span>{/if}
        </div>
      </div>

      <div class="controls">
        <label>
          <span class="cap" lang={i18n.lang}>{t('opener.time')}</span>
          <input class="field num" type="time" value={hm(o.minute)} onchange={(e) => setClock(e.currentTarget.value)} />
        </label>
        <div role="group" aria-label={t('opener.days')}>
          <span class="cap" lang={i18n.lang}>{t('opener.days')}</span>
          <span class="days">
            {#each weekdayNames() as name, i (i)}
              {@const on = o.days.includes(i + 1)}
              <button class="day" class:on aria-pressed={on} onclick={() => setDay(i + 1, !on)}>{name}</button>
            {/each}
          </span>
        </div>
        <span class="spacer"></span>
        <button class="btn try" disabled={busy || info.running || !info.available} onclick={now}>
          <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7 5l12 7-12 7z" /></svg>
          {busy || info.running ? t('opener.running') : t('opener.now')}
        </button>
      </div>
      {#if !info.available}<p class="subtle small plain">{t('opener.unavailable')}</p>{/if}
      {#if error}<p class="error small plain" role="alert">{t('common.error', { e: error })}</p>{/if}

      {#if info.runs.length}
        <div class="runs">
          <span class="cap" lang={i18n.lang}>{t('opener.runs')}</span>
          <div class="inset">
            {#each info.runs.slice(0, 3) as r (r.at_ms)}
              <div class="row run">
                <span class="dot {r.outcome}"></span>
                <span class="at num">{fmtWhen(r.at_ms)}</span>
                <span>
                  {#if r.outcome === 'sent'}
                    {t('opener.run.sent')}<span class="subtle">{` · ${t('opener.run.tokens', { tokens: fmtCompact(r.tokens ?? 0) })}`}</span>
                  {:else if r.outcome === 'open'}
                    {t('opener.run.open')}<span class="subtle">{` · ${t('opener.run.notSent')}`}</span>
                  {:else}
                    <span class="failed">{t('opener.run.failed', { why: why(r.error ?? '') })}</span>
                  {/if}
                </span>
                <span class="window">{window(r)}</span>
              </div>
            {/each}
          </div>
        </div>
      {/if}
    {/if}
    <p class="note">
      <svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 7.5v.5" /></svg>
      <span>{t('opener.cost')}</span>
    </p>
  </AutoCard>
{/if}

<style>
  .line {
    display: grid;
    gap: 8px;
    padding: 14px 18px 10px;
  }
  .workday {
    position: relative;
    height: 25px;
    font-size: 11px;
    color: var(--ink-3);
  }
  /* nine to six, as a bracket over the line */
  .workday::before {
    content: '';
    position: absolute;
    left: 37.5%;
    width: 37.5%;
    bottom: 0;
    height: 6px;
    box-sizing: border-box;
    border: 1px solid var(--hairline-strong);
    border-bottom: 0;
    border-radius: 4px 4px 0 0;
  }
  .workday span {
    position: absolute;
    left: 56.25%;
    top: 0;
    transform: translateX(-50%);
    white-space: nowrap;
  }
  .track {
    position: relative;
    height: 34px;
    border-radius: 9px;
    overflow: hidden;
    background: var(--cc-track);
  }
  .band {
    position: absolute;
    top: 0;
    bottom: 0;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    padding-left: 12px;
    overflow: hidden;
    white-space: nowrap;
    font-size: 12px;
    font-weight: 600;
    background: var(--tint);
    color: var(--tint-ink);
    border: 1px dashed var(--tint-edge);
    border-radius: 3px;
  }
  .band.first {
    background: var(--accent);
    color: var(--accent-ink);
    border: 0;
  }
  .scale {
    position: relative;
    height: 32px;
    font-size: 11px;
    color: var(--ink-3);
  }
  .edge {
    position: absolute;
    left: 0;
  }
  .edge.end {
    left: auto;
    right: 0;
  }
  .mark {
    position: absolute;
    display: grid;
    justify-items: center;
    transform: translateX(-50%);
    white-space: nowrap;
  }
  .mark b {
    font-weight: 600;
    color: var(--ink);
  }
  .mark span {
    color: var(--tint-ink);
  }
  .controls {
    display: flex;
    align-items: flex-end;
    flex-wrap: wrap;
    gap: 14px 22px;
  }
  .controls label,
  .controls div {
    display: grid;
    gap: 6px;
  }
  .controls .field {
    height: 34px;
    font-size: 14px;
    font-weight: 600;
    border-radius: 9px;
  }
  .days {
    display: flex;
    gap: 5px;
  }
  .day {
    min-width: 44px;
    height: 34px;
    padding: 0 8px;
    border-radius: 9px;
    border: 0.5px solid var(--hairline-strong);
    background: transparent;
    color: var(--ink-3);
    font-size: 12.5px;
    font-weight: 500;
  }
  .day.on {
    background: var(--tint);
    border-color: var(--tint-edge);
    color: var(--tint-ink);
    font-weight: 600;
  }
  .try {
    height: 34px;
    border-radius: 9px;
  }
  .runs {
    display: grid;
    gap: 8px;
  }
  .row.run {
    grid-template-columns: 8px minmax(72px, auto) minmax(0, 1fr) auto;
    gap: 12px;
    padding: 10px 16px;
    font-size: 13px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ink-3);
  }
  .dot.sent {
    background: var(--good-ink);
  }
  .dot.failed {
    background: var(--bad-ink);
  }
  .at,
  .window {
    color: var(--ink-2);
  }
  .at {
    font-weight: 500;
  }
  .failed,
  .error {
    color: var(--bad-ink);
  }
  .plain {
    margin: 0;
  }
  @media (max-width: 720px) {
    .row.run {
      grid-template-columns: 8px minmax(0, 1fr);
      row-gap: 2px;
    }
    .row.run > :nth-child(n + 3) {
      grid-column: 2;
    }
  }
</style>
