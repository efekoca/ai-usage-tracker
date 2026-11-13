<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, type ResumeInfo } from '../lib/api'
  import { cacheClient } from '../lib/caches'
  import { fmtClock, fmtDuration, fmtWhen, has, t, windowLabel } from '../lib/i18n.svelte'
  import AutoCard from './AutoCard.svelte'

  type Waiting = ResumeInfo['waiting'][number]

  let info = $state<ResumeInfo | null>(null)
  let asking = $state(false)
  let busy = $state(false)
  let error = $state('')
  // the time left is told to the minute
  let now = $state(Date.now())

  const load = () =>
    api
      .limitResumeInfo()
      .then((i) => (info = i))
      .catch(() => {})
  $effect(() => {
    void app.tick
    load()
  })
  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), 20_000)
    return () => clearInterval(timer)
  })

  const on = $derived(!!app.settings?.limit_resume && !!info?.installed)

  async function set(v: boolean) {
    busy = true
    error = ''
    try {
      info = await api.setLimitResume(v)
      // the app sends the saved settings too; this keeps the switch from waiting for them
      if (app.settings) app.settings.limit_resume = v
      asking = false
    } catch (e) {
      // the question stays open and says why, e.g. a settings file that is not valid JSON
      error = String(e)
    } finally {
      busy = false
    }
  }
  // the hook is added only once the user agrees
  function toggle(v: boolean) {
    if (v && !info?.installed) {
      error = ''
      asking = true
    } else set(v)
  }
  async function cancel(session: string) {
    await api.cancelLimitResume(session).catch(() => {})
    await load()
  }

  const project = (c: { project: string }) => c.project || t('projects.hidden')
  const name = (c: { project: string; branch: string | null }) => [project(c), c.branch].filter(Boolean).join(' · ')
  const stopped = (w: string) => (has(`limits.resume.stopped.${w}`) ? t(`limits.resume.stopped.${w}`) : t('limits.resume.stopped', { window: windowLabel(w) }))
  // today's times are a clock time; another day's carries its day
  const clock = (ms: number) => (new Date(ms).toDateString() === new Date(now).toDateString() ? fmtClock(ms) : fmtWhen(ms))
  const scheduled = (w: Waiting) => !w.held && w.resumes_ms !== null
  /** How much of the wait is over, 0 to 1. */
  function waited(w: Waiting): number {
    const all = (w.resumes_ms ?? 0) - w.stopped_ms
    return all > 0 ? Math.min(1, Math.max(0, (now - w.stopped_ms) / all)) : 1
  }
  /** A wedge from twelve o'clock, as much of the circle as `share`. */
  function wedge(share: number): string {
    if (share >= 0.999) return 'M11 5.5a5.5 5.5 0 1 1 0 11a5.5 5.5 0 1 1 0-11Z'
    const a = share * 2 * Math.PI
    return `M11 11V5.5A5.5 5.5 0 ${share > 0.5 ? 1 : 0} 1 ${(11 + 5.5 * Math.sin(a)).toFixed(2)} ${(11 - 5.5 * Math.cos(a)).toFixed(2)}Z`
  }
  const tag = $derived(on && info?.waiting.length ? t('limits.resume.count', { n: info.waiting.length }) : '')
</script>

{#snippet icon()}
  <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 3-6.7" /><path d="M3 4v5h5" /><path d="M10 9l5 3-5 3z" fill="currentColor" stroke="none" /></svg>
{/snippet}

{#if info}
  <AutoCard title={t('limits.resume.title')} help={t('limits.resume.help')} {tag} {on} disabled={busy} onchange={toggle} {icon}>
    {#if asking}
      <div class="inset ask" role="group" aria-label={t('limits.resume.ask.title')}>
        <div class="ask-head">
          <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5M12 12v5M9.5 14.5h5" /></svg>
          <div>
            <b>{t('limits.resume.ask.title')}</b>
            <code>{info.settings_file}</code>
          </div>
        </div>
        <ul>
          <li>{t('limits.resume.ask.runs')}</li>
          <li>{t('limits.resume.ask.only')}</li>
          <li>{t('limits.resume.ask.off')}</li>
        </ul>
        {#if error}<p class="error small" role="alert">{t('refresh.consent.failed', { why: error })}</p>{/if}
        <div class="actions">
          <button class="btn" disabled={busy} onclick={() => (asking = false)}>{t('common.cancel')}</button>
          <button class="btn primary" disabled={busy} onclick={() => set(true)}>{t('limits.resume.ok')}</button>
        </div>
      </div>
    {:else if error}
      <p class="error small" role="alert">{t('common.error', { e: error })}</p>
    {/if}
    {#if on}
      {#if info.waiting.length || info.resumed.length}
        <div class="inset">
          {#each info.waiting as w (w.session_id)}
            <div class="row waiting">
              {#if scheduled(w)}
                <svg class="state" role="img" aria-label={t('limits.resume.state.waiting')} width="22" height="22" viewBox="0 0 22 22"><circle cx="11" cy="11" r="9" fill="none" stroke="var(--accent)" stroke-width="2" /><path d={wedge(waited(w))} fill="var(--accent)" /></svg>
              {:else}
                <svg class="state" role="img" aria-label={t('limits.resume.state.unknown')} width="22" height="22" viewBox="0 0 22 22"><circle cx="11" cy="11" r="9" fill="none" stroke="var(--ink-3)" stroke-width="2" stroke-dasharray="2.7 2.95" stroke-linecap="round" /></svg>
              {/if}
              <div class="who">
                <span class="name"><b>{project(w)}</b>{#if w.branch}<span class="subtle">{` · ${w.branch}`}</span>{/if}</span>
                <span class="chips">
                  {#if cacheClient(w.client)}<span class="chip">{cacheClient(w.client)}</span>{/if}
                  {#if w.window}<span class="chip">{stopped(w.window)}</span>{/if}
                </span>
              </div>
              <div class="when">
                {#if w.held}
                  <span class="subtle small">{t('limits.resume.held')}</span>
                {:else if w.resumes_ms === null}
                  <span class="small">{t('limits.resume.unknown')}</span>
                  <span class="subtle small">{t('limits.resume.stoppedAgo', { span: fmtDuration(now - w.stopped_ms) })}</span>
                {:else}
                  <b class="clock num">{clock(w.resumes_ms)}</b>
                  <span class="subtle small">{w.resumes_ms <= now ? t('limits.resume.soon') : t('limits.resume.in', { span: fmtDuration(w.resumes_ms - now) })}</span>
                {/if}
              </div>
              <button class="btn small" aria-label="{t('limits.resume.cancel')}: {name(w)}" onclick={() => cancel(w.session_id)}>{t('limits.resume.cancel')}</button>
            </div>
          {/each}
          {#each info.resumed as r (r.session_id + r.at_ms)}
            <div class="row done">
              <svg class="state" role="img" aria-label={t('limits.resume.state.done')} width="22" height="22" viewBox="0 0 22 22"><circle cx="11" cy="11" r="10" fill="var(--good-ink)" /><path d="M6.8 11.3l2.8 2.8 5.6-6" fill="none" stroke="var(--surface)" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" /></svg>
              <div class="who">
                <span class="name"><b>{project(r)}</b>{#if r.branch}<span class="subtle">{` · ${r.branch}`}</span>{/if}</span>
                <span class="subtle small">{t('limits.resume.went')}</span>
              </div>
              <div class="when">
                <b class="clock num good">{clock(r.at_ms)}</b>
                <span class="subtle small">{t('limits.resume.done')}</span>
              </div>
            </div>
          {/each}
        </div>
      {:else}
        <p class="subtle small none">{t('limits.resume.none')}</p>
      {/if}
      <div class="notes">
        <p class="note">
          <svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="13" rx="2" /><path d="M8 21h8M12 17v4" /></svg>
          <span>{t('limits.resume.note.awake')}</span>
        </p>
        <p class="note">
          <svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3l8 3v6c0 4.5-3.2 8-8 9-4.8-1-8-4.5-8-9V6z" /></svg>
          <span>{t('limits.resume.note.permissions')}</span>
        </p>
        <p class="note">
          <svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20l4-1 11-11-3-3L5 16z" /></svg>
          <span>{t('limits.resume.note.ends')}</span>
        </p>
      </div>
    {/if}
  </AutoCard>
{/if}

<style>
  .row.waiting {
    grid-template-columns: 22px minmax(0, 1fr) auto auto;
  }
  .row.done {
    grid-template-columns: 22px minmax(0, 1fr) auto;
  }
  .who {
    display: grid;
    gap: 5px;
    min-width: 0;
  }
  .name {
    font-size: 14px;
  }
  .name .subtle {
    font-weight: 500;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    padding: 1px 8px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--ink) 8%, transparent);
    color: var(--ink-2);
    font-size: 11.5px;
  }
  .when {
    display: grid;
    gap: 1px;
    justify-items: end;
    text-align: right;
  }
  .clock {
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .good {
    color: var(--good-ink);
  }
  .notes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 10px;
  }
  .none {
    margin: 0;
  }
  .ask {
    display: grid;
    gap: 14px;
    padding: 18px 20px;
    border-color: var(--hairline-strong);
  }
  .ask-head {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  .ask-head svg {
    flex: none;
    margin-top: 1px;
    color: var(--tint-ink);
  }
  .ask-head div {
    display: grid;
    gap: 8px;
    justify-items: start;
    min-width: 0;
  }
  .ask-head b {
    font-size: 14px;
    font-weight: 600;
  }
  code {
    max-width: 100%;
    padding: 3px 9px;
    border-radius: 7px;
    background: var(--surface-2);
    color: var(--ink-2);
    font: 12px ui-monospace, 'SF Mono', Menlo, Consolas, monospace;
    overflow-wrap: anywhere;
  }
  ul {
    margin: 0;
    padding: 0 0 0 30px;
    list-style: none;
    display: grid;
    gap: 7px;
    color: var(--ink-2);
    font-size: 13px;
  }
  li {
    display: flex;
    gap: 9px;
  }
  li::before {
    content: '•';
    color: var(--tint-ink);
  }
  .actions {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    justify-self: end;
    gap: 8px;
  }
  .actions .btn {
    justify-content: center;
  }
  .error {
    margin: 0;
    color: var(--bad-ink);
  }
  @media (max-width: 720px) {
    .row.waiting {
      grid-template-columns: 22px minmax(0, 1fr) auto;
    }
    .row.waiting .btn {
      grid-column: 2 / -1;
      justify-self: start;
    }
  }
</style>
