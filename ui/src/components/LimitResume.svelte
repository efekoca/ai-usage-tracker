<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, type ResumeInfo } from '../lib/api'
  import { cacheClient } from '../lib/caches'
  import { clockCase, fmtDate, has, t, windowLabel } from '../lib/i18n.svelte'
  import Toggle from './Toggle.svelte'

  let info = $state<ResumeInfo | null>(null)
  let asking = $state(false)
  let busy = $state(false)
  let error = $state('')

  const load = () =>
    api
      .limitResumeInfo()
      .then((i) => (info = i))
      .catch(() => {})
  $effect(() => {
    void app.tick
    load()
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

  const name = (c: { project: string; branch: string | null }) => [c.project || t('projects.hidden'), c.branch].filter(Boolean).join(' · ')
  function when(ms: number, key: string): string {
    const [d, today] = [new Date(ms), new Date()]
    const at = clockCase(ms, 'at')
    return d.toDateString() === today.toDateString() ? t(key, { at }) : t(`${key}Day`, { day: fmtDate(ms, 'weekday'), at })
  }
  const stopped = (w: string) => (has(`limits.resume.stopped.${w}`) ? t(`limits.resume.stopped.${w}`) : t('limits.resume.stopped', { window: windowLabel(w) }))
  function status(w: ResumeInfo['waiting'][number]): string {
    if (w.held) return t('limits.resume.held')
    if (w.resumes_ms === null) return t('limits.resume.unknown')
    return w.resumes_ms <= Date.now() ? t('limits.resume.soon') : when(w.resumes_ms, 'limits.resume.at')
  }
</script>

{#if info}
  <section class="card resume">
    <div class="head">
      <div>
        <h2>{t('limits.resume.title')}</h2>
        <p class="subtle small">{t('limits.resume.help')}</p>
      </div>
      <Toggle checked={on} disabled={busy} label={t('limits.resume.title')} onchange={toggle} />
    </div>
    {#if asking}
      <div class="ask" role="group" aria-label={t('limits.resume.title')}>
        <p class="small">{t('limits.resume.consent', { file: info.settings_file })}</p>
        <p class="subtle small">{t('limits.resume.conditions')}</p>
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
      {#each info.waiting as w (w.session_id)}
        <div class="list-row">
          <span class="who">
            <b>{name(w)}</b>
            <span class="subtle small">{[cacheClient(w.client), w.window ? stopped(w.window) : ''].filter(Boolean).join(' · ')}</span>
          </span>
          <span class="spacer"></span>
          <span class="small" class:subtle={w.held || w.resumes_ms === null}>{status(w)}</span>
          <button class="btn ghost small" aria-label="{t('limits.resume.cancel')}: {name(w)}" onclick={() => cancel(w.session_id)}>{t('limits.resume.cancel')}</button>
        </div>
      {:else}
        {#if info.resumed.length === 0}<p class="subtle small none">{t('limits.resume.none')}</p>{/if}
      {/each}
      {#each info.resumed as r (r.session_id + r.at_ms)}
        <div class="list-row done">
          <span class="who"><b>{name(r)}</b></span>
          <span class="spacer"></span>
          <span class="subtle small">{when(r.at_ms, 'limits.resume.done')}</span>
        </div>
      {/each}
      <p class="subtle small note">{t('limits.resume.conditions')}</p>
    {/if}
  </section>
{/if}

<style>
  .resume {
    margin-bottom: 16px;
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    justify-content: space-between;
  }
  .head p {
    margin: 4px 0 0;
  }
  .ask {
    display: grid;
    gap: 8px;
    margin-top: 14px;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--surface-2);
    border: 0.5px solid var(--hairline);
  }
  .ask p {
    margin: 0;
    line-height: 1.5;
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
    color: var(--bad-ink);
  }
  .list-row:first-of-type {
    margin-top: 10px;
  }
  .who {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .none,
  .note {
    margin: 10px 0 0;
  }
</style>
