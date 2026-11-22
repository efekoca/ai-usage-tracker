<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, type HandoffAgent, type HandoffInfo } from '../lib/api'
  import { backToSwitch } from '../lib/focus'
  import { fmtClock, fmtWhen, has, t } from '../lib/i18n.svelte'
  import AutoCard from './AutoCard.svelte'
  import Icon from './Icon.svelte'
  import HooksOff from './HooksOff.svelte'
  import Toggle from './Toggle.svelte'

  let info = $state<HandoffInfo | null>(null)
  // which tools the consent panel is for
  let asking = $state<HandoffAgent[]>([])
  let how = $state(false)
  let busy = $state(false)
  let error = $state('')
  let failed = $state<{ id: string; why: string } | null>(null)

  // a read that comes back after a newer read or a change is dropped
  let seq = 0
  const load = () => {
    const mine = ++seq
    api
      .handoffInfo()
      .then((i) => mine === seq && (info = i))
      .catch(() => {})
  }
  const took = (i: HandoffInfo) => {
    seq++
    info = i
  }
  $effect(() => {
    void app.tick
    load()
  })

  const claudeOn = $derived(!!app.settings?.limit_handoff && !!info?.installed)
  const codexOn = $derived(!!app.settings?.codex_handoff && !!info?.codex_installed)
  const on = $derived(claudeOn || codexOn)
  const needs = (tool: HandoffAgent) => (tool === 'claude' ? !info?.installed : !info?.codex_installed)

  // the user's own Codex hooks that moved up when ours went: Codex asks about them again
  let moved = $state(0)
  async function apply(change: Partial<Record<HandoffAgent, boolean>>) {
    busy = true
    error = ''
    moved = 0
    try {
      if (change.claude !== undefined) {
        took(await api.setLimitHandoff(change.claude))
        if (app.settings) app.settings.limit_handoff = change.claude
      }
      if (change.codex !== undefined) {
        const changed = await api.setCodexHandoff(change.codex)
        took(changed)
        moved = changed.codex_moved
        if (app.settings) app.settings.codex_handoff = change.codex
      }
      asking = []
    } catch (e) {
      error = String(e)
    } finally {
      busy = false
    }
  }
  // the hooks and the write rule are added only once the user agrees
  // the switch the question came from gets the keyboard back
  let opener: HTMLElement | null = null
  function ask(tools: HandoffAgent[]) {
    if (tools.some(needs)) {
      error = ''
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
      asking = tools
    } else apply(Object.fromEntries(tools.map((tool) => [tool, true])))
  }
  const toggleAll = (v: boolean) => (v ? ask(info?.codex_here ? ['claude', 'codex'] : ['claude']) : apply({ claude: false, codex: false }))
  const toggleTool = (tool: HandoffAgent, v: boolean) => (v ? ask([tool]) : apply({ [tool]: false }))

  async function run(id: string, job: (id: string) => Promise<void>) {
    failed = null
    try {
      await job(id)
    } catch (e) {
      const why = String(e)
      failed = { id, why: has(`limits.handoff.failed.${why}`) ? t(`limits.handoff.failed.${why}`) : t('common.error', { e: why }) }
    }
  }

  const project = (n: { project: string }) => n.project || t('projects.hidden')
  const toolName = (a: HandoffAgent) => (a === 'codex' ? 'Codex' : 'Claude')
  // today's times are a clock time; another day's carries its day
  const clock = (ms: number) => (new Date(ms).toDateString() === new Date().toDateString() ? fmtClock(ms) : fmtWhen(ms))
  const tag = $derived(on && info?.notes.length ? t('limits.handoff.count', { n: info.notes.length }) : '')
  const codexState = $derived(!info?.codex_here ? 'missing' : codexOn ? (info.codex_seen ? 'on' : 'waiting') : 'off')
  // the user goes on in the other tool than the one that wrote the note
  const go = (a: HandoffAgent) =>
    a === 'codex' ? { label: t('limits.handoff.claude'), found: !!info?.claude_found, missing: 'claude_not_found' } : { label: t('limits.handoff.codex'), found: !!info?.codex_found, missing: 'codex_not_found' }
</script>

{#snippet icon()}
  <svg width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" /><path d="M9 13h6M9 17h4" /></svg>
{/snippet}

{#snippet about()}
  <button class="how" aria-expanded={how} aria-controls="handoff-how" onclick={() => (how = !how)}>
    {t('limits.resume.how')}<span class="chev" class:up={how}><Icon name="chevron" size={11} /></span>
  </button>
  {#if how}
    <ul class="how-list" id="handoff-how">
      <li>
        <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></svg>
        <span>{t('limits.handoff.note.status')}</span>
      </li>
      <li>
        <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20l4-1 11-11-3-3L5 16z" /></svg>
        <span>{t('limits.handoff.note.once')}</span>
      </li>
      <li>
        <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" /></svg>
        <span>{t('limits.handoff.note.keep')}</span>
      </li>
    </ul>
  {/if}
{/snippet}

{#snippet head(title: string, file: string)}
  <div class="ask-head">
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5M12 12v5M9.5 14.5h5" /></svg>
    <div>
      <b>{title}</b>
      <code>{file}</code>
    </div>
  </div>
{/snippet}

{#if info}
  <AutoCard title={t('limits.handoff.title')} help={t('limits.handoff.help')} {tag} {on} disabled={busy} onchange={toggleAll} {icon} {about}>
    {#if asking.length}
      <div class="inset ask" role="group" aria-label={t('limits.handoff.title')}>
        {#if asking.includes('claude')}
          <section class="part claude">
            {@render head(t('limits.handoff.ask.title'), info.settings_file)}
            <ul>
              <li>{t('limits.handoff.ask.runs')}</li>
              <li>{t('limits.handoff.ask.intro')}</li>
              <li>
                <span class="folder">{t('limits.handoff.ask.folder')}<code>{info.folder}</code></span>
              </li>
            </ul>
          </section>
        {/if}
        {#if asking.includes('codex')}
          <section class="part codex">
            {@render head(t('limits.handoff.ask.codex.title'), info.codex_hooks_file)}
            <ul>
              <li>{t('limits.handoff.ask.codex.runs')}</li>
              <li>{t('limits.handoff.ask.codex.intro')}</li>
              <li>{t('limits.handoff.ask.codex.trust')}</li>
              <li>
                <span class="folder">{t('limits.handoff.ask.codex.folder')}<code>{info.codex_folder}</code></span>
              </li>
              <li>{t('limits.handoff.ask.codex.same')}</li>
            </ul>
          </section>
        {/if}
        <ul class="common">
          <li>{t('limits.handoff.ask.only')}</li>
          <li>{t('limits.handoff.ask.off')}</li>
        </ul>
        {#if error}<p class="error small" role="alert">{t('refresh.consent.failed', { why: error })}</p>{/if}
        <div class="actions">
          <button class="btn" disabled={busy} onclick={(e) => backToSwitch(e, () => ((asking = []), (error = '')), () => !asking.length, opener)}>{t('common.cancel')}</button>
          <button class="btn primary" disabled={busy} onclick={(e) => backToSwitch(e, () => apply(Object.fromEntries(asking.map((tool) => [tool, true]))), () => !asking.length, opener)}>{t('limits.handoff.ok')}</button>
        </div>
      </div>
    {:else if error}
      <p class="error small" role="alert">{t('common.error', { e: error })}</p>
    {/if}
    {#if moved}<p class="moved subtle small" role="status">{t('limits.handoff.codexMoved', { n: moved })}</p>{/if}
    {#if claudeOn}<HooksOff setting={info.hooks_off} />{/if}
    {#if on}
      <div class="inset tools" role="group" aria-label={t('limits.handoff.tools')}>
        <div class="row tool">
          <b>Claude Code</b>
          <span class="subtle small">{t(claudeOn ? 'limits.handoff.tool.on' : 'limits.handoff.tool.off')}</span>
          <Toggle small checked={claudeOn} disabled={busy} label="Claude Code" onchange={(v) => toggleTool('claude', v)} />
        </div>
        <div class="row tool">
          <b>Codex</b>
          <span class="subtle small">{t(`limits.handoff.tool.${codexState}`)}</span>
          <Toggle small checked={codexOn} disabled={busy || !info.codex_here} label="Codex" onchange={(v) => toggleTool('codex', v)} />
        </div>
      </div>
      <div class="inset">
        {#each info.notes as n (n.id)}
          {@const next = go(n.agent)}
          <div class="row note">
            <svg class="state" role="img" aria-label={t('limits.handoff.state')} width="22" height="22" viewBox="0 0 22 22"><rect x="4" y="2.5" width="14" height="17" rx="2.5" fill="none" stroke="var(--accent)" stroke-width="2" /><path d="M7.5 8h7M7.5 11.5h7M7.5 15h4" stroke="var(--accent)" stroke-width="1.8" stroke-linecap="round" /></svg>
            <div class="who">
              <span class="name"><b>{project(n)}</b></span>
              <span class="subtle small">{toolName(n.agent)} · {t('limits.handoff.written', { when: clock(n.written_ms) })}</span>
              {#if failed?.id === n.id}<span class="error small" role="alert">{failed.why}</span>{/if}
            </div>
            <div class="buttons">
              <button class="btn small" aria-label="{t('limits.handoff.open')}: {project(n)}" onclick={() => run(n.id, api.openHandoffNote)}>{t('limits.handoff.open')}</button>
              <button class="btn small primary" disabled={!next.found} title={next.found ? undefined : t(`limits.handoff.failed.${next.missing}`)} aria-label="{next.label}: {project(n)}" onclick={() => run(n.id, api.continueHandoff)}>{next.label}</button>
            </div>
          </div>
        {:else}
          <div class="empty-line">
            <svg aria-hidden="true" width="22" height="22" viewBox="0 0 22 22"><circle cx="11" cy="11" r="8" fill="none" stroke="var(--ink-3)" stroke-width="2" stroke-dasharray="2.7 2.95" stroke-linecap="round" /></svg>
            {t('limits.handoff.none')}
          </div>
        {/each}
      </div>
    {/if}
  </AutoCard>
{/if}

<style>
  .row.note {
    grid-template-columns: 22px minmax(0, 1fr) auto;
  }
  .row.tool {
    grid-template-columns: minmax(0, 1fr) auto auto;
    padding: 10px 16px;
    font-size: 13.5px;
  }
  .row.tool b {
    font-weight: 600;
  }
  .who {
    display: grid;
    gap: 3px;
    min-width: 0;
  }
  .name {
    font-size: 14px;
  }
  .buttons {
    display: flex;
    gap: 8px;
  }
  .empty-line {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 11px 16px;
    font-size: 13px;
    color: var(--ink-2);
  }
  .empty-line svg {
    flex: none;
  }
  .how {
    justify-self: start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 2px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--tint-ink);
    font: inherit;
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
  }
  .how:hover {
    color: var(--ink);
  }
  .how:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .chev {
    display: inline-flex;
    transform: rotate(90deg);
    transition: transform var(--dur) var(--ease);
  }
  .chev.up {
    transform: rotate(-90deg);
  }
  .how-list {
    margin: 4px 0 0;
    padding: 0;
    max-width: 640px;
    list-style: none;
    display: grid;
    gap: 7px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--ink-3);
  }
  .how-list li {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .how-list svg {
    flex: none;
    margin-top: 2px;
  }
  .ask {
    display: grid;
    gap: 14px;
    padding: 18px 20px;
    border-color: var(--hairline-strong);
  }
  .part {
    display: grid;
    gap: 12px;
  }
  .part + .part {
    padding-top: 14px;
    border-top: 1px solid var(--hairline);
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
  .folder {
    display: grid;
    gap: 6px;
    justify-items: start;
    min-width: 0;
  }
  .ask ul {
    margin: 0;
    padding: 0 0 0 30px;
    list-style: none;
    display: grid;
    gap: 7px;
    color: var(--ink-2);
    font-size: 13px;
  }
  .ask li {
    display: flex;
    gap: 9px;
    min-width: 0;
  }
  .ask li::before {
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
  .moved {
    margin: 0;
  }
  .error {
    margin: 0;
    color: var(--bad-ink);
  }
  @media (max-width: 720px) {
    .row.note {
      grid-template-columns: 22px minmax(0, 1fr);
    }
    .buttons {
      grid-column: 2 / -1;
      flex-wrap: wrap;
    }
  }
</style>
