<script lang="ts">
  import { onMount } from 'svelte'
  import { app } from '../lib/store.svelte'
  import { api, type CaptureKind, type CaptureStatus } from '../lib/api'
  import { fmtDuration, fmtInt, t } from '../lib/i18n.svelte'
  import Toggle from './Toggle.svelte'
  import Select from './Select.svelte'
  import Icon from './Icon.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'
  import SignInSteps from './SignInSteps.svelte'

  let st = $state<CaptureStatus | null>(null)
  let busy = $state<string | null>(null)
  let note = $state<Record<string, { ok: boolean; text: string }>>({})
  let now = $state(Date.now())

  // the interval, data changes and toggles all load; only the newest answer is shown
  let gen = 0
  async function load() {
    const mine = ++gen
    try {
      const s = await api.captureStatus()
      if (mine === gen) {
        st = s
        now = Date.now()
      }
    } catch {
      /* the next tick tries again */
    }
  }
  onMount(() => {
    load()
    const id = setInterval(load, 10_000)
    return () => clearInterval(id)
  })
  $effect(() => {
    void app.tick
    load()
  })

  function explain(kind: string, raw: string, ok: boolean): string {
    const [code, ...rest] = raw.split(':')
    const key = `cap.msg.${code}`
    const s = t(key, { detail: rest.join(':') })
    return s === key ? (ok ? t('settings.data.done') : raw) : s
  }

  async function toggle(kind: CaptureKind, on: boolean) {
    busy = kind
    try {
      const r = await api.setCapture(kind, on)
      note[kind] = { ok: true, text: explain(kind, r, true) }
    } catch (e) {
      note[kind] = { ok: false, text: explain(kind, String(e), false) }
    } finally {
      busy = null
      await load()
    }
  }
  async function install(kind: 'claude' | 'codex' | 'antigravity') {
    busy = kind
    try {
      note[kind] = { ok: true, text: explain(kind, await api.installCli(kind), true) }
    } catch (e) {
      note[kind] = { ok: false, text: explain(kind, String(e), false) }
    } finally {
      busy = null
      await load()
    }
  }
  // Claude's usage service answers at most every 5 minutes
  const STEPS = { claude: [5, 10, 15, 30, 60], codex: [1, 2, 5, 10, 15, 30, 60], antigravity: [1, 2, 5, 10, 15, 30, 60] }
  const every = (kind: 'claude' | 'codex' | 'antigravity') => app.settings?.capture[`${kind}_poll_minutes`] ?? 5
  function steps(kind: 'claude' | 'codex' | 'antigravity') {
    const list = STEPS[kind].includes(every(kind)) ? STEPS[kind] : [...STEPS[kind], every(kind)].sort((a, b) => a - b)
    return list.map((n) => ({ value: String(n), label: t('cap.every', { n }) }))
  }
  // per reader, only the newest choice's answer counts, and it touches only the capture settings;
  // a failed save redraws the list so it shows the interval still in use
  let unsaved = $state(0)
  let everyError = $state<Record<string, string>>({})
  const asked: Record<string, number> = {}
  async function setEvery(kind: 'claude' | 'codex' | 'antigravity', minutes: number) {
    const mine = (asked[kind] = (asked[kind] ?? 0) + 1)
    try {
      const s = await api.setReadInterval(kind, minutes)
      if (mine !== asked[kind]) return
      if (app.settings) app.settings = { ...app.settings, capture: s.capture }
      delete everyError[kind]
    } catch (e) {
      if (mine !== asked[kind]) return
      everyError[kind] = explain(kind, String(e), false)
      unsaved++
    }
  }
  const GUIDES = {
    claude: 'https://code.claude.com/docs/en/setup',
    codex: 'https://learn.chatgpt.com/docs/codex/cli',
    antigravity: 'https://antigravity.google/docs/getting-started?tab=cli',
  }
  const SIGNED_OUT = { claude: 'claude_no_plan_limits', codex: 'codex_not_signed_in', antigravity: 'agy_not_signed_in' } as const
  const signIn = (kind: string) =>
    !!st && (kind === 'claude' || kind === 'codex' || kind === 'antigravity') && !missing(kind) && st[kind].last_error === SIGNED_OUT[kind]
  const missing = (kind: string) =>
    !!st && ((kind === 'claude' && !st.claude_candidates_found) || (kind === 'codex' && !st.codex_candidates_found) || (kind === 'antigravity' && !st.antigravity_candidates_found))
  const ago = (ms: number | null | undefined) => (ms ? t('cap.ago', { t: fmtDuration(now - ms) }) : t('cap.never'))
</script>

{#snippet installRow(kind: 'claude' | 'codex' | 'antigravity')}
  <span class="install">
    <button class="btn small primary" disabled={busy !== null} onclick={() => install(kind)}><Icon name="download" size={13} />{t(`cap.${kind}.install`)}</button>
    <button class="btn small ghost" onclick={() => api.openUrl(GUIDES[kind])}><Icon name="external" size={13} />{t('cap.install.guide')}</button>
  </span>
{/snippet}

<section class="card">
  <div class="head">
    <h2>{t('cap.title')}</h2>
    <AccuracyBadge kind="captured" />
  </div>
  <p class="subtle small lead">{t('cap.lead')}</p>

  {#if st}
    {#each [['claude', st.claude_poll], ['codex', st.codex_poll], ['antigravity', st.antigravity_poll], ['statusline', st.statusline], ['otel', st.otel]] as const as [kind, on] (kind)}
      <div class="method" class:on>
        <div class="top">
          <div class="txt">
            <b>{t(`cap.${kind}.name`)}</b>
            <span class="subtle small">{t(`cap.${kind}.what`)}</span>
          </div>
          <Toggle checked={on} label={t(`cap.${kind}.name`)} disabled={busy !== null} onchange={(v) => toggle(kind, v)} />
        </div>
        <dl>
          <dt>{t('cap.changes')}</dt>
          <dd>
            {#if kind === 'codex' || kind === 'claude' || kind === 'antigravity'}{t(`cap.${kind}.changes`)}
            {:else}{t(`cap.${kind}.changes`)} <code>{st.settings_file}</code>{/if}
          </dd>
          {#if kind === 'claude' || kind === 'codex' || kind === 'antigravity'}
            <dt>{t('cap.interval')}</dt>
            <dd>
              {#key unsaved}
                <Select label="{t(`cap.${kind}.name`)} · {t('cap.interval')}" value={String(every(kind))} options={steps(kind)} onchange={(v) => setEvery(kind, Number(v))} />
              {/key}
              {#if kind === 'claude'}<span class="subtle small">{t('cap.claude.idle')}</span>{/if}
              {#if everyError[kind]}<Icon name="warning" size={13} /> <span class="msg">{everyError[kind]}</span>{/if}
            </dd>
          {/if}
          <dt>{t('cap.status')}</dt>
          <dd>
            {#if busy === kind}
              <span class="spin" aria-hidden="true"></span> {t(missing(kind) ? `cap.${kind}.installing` : 'cap.working')}
            {:else if kind === 'claude'}
              {#if !st.claude_candidates_found}<span class="muted">{t('cap.claude.notInstalled')}</span>{@render installRow('claude')}
              {:else if on && st.claude.last_error}<Icon name="warning" size={13} /> <span class="msg">{explain('claude', st.claude.last_error, false)}</span>
              {:else if on}<Icon name="check" size={13} /> {t('cap.codex.ok', { t: ago(st.claude.last_ok_ms) })}
              {:else}{t('common.off')}{/if}
            {:else if kind === 'codex'}
              {#if !st.codex_candidates_found}<span class="muted">{t('cap.codex.notInstalled')}</span>{@render installRow('codex')}
              {:else if on && st.codex.last_error}<Icon name="warning" size={13} /> <span class="msg">{explain('codex', st.codex.last_error, false)}</span>
              {:else if on}<Icon name="check" size={13} /> {t('cap.codex.ok', { t: ago(st.codex.last_ok_ms) })}
              {:else}{t('common.off')}{/if}
            {:else if kind === 'antigravity'}
              {#if !st.antigravity_candidates_found}<span class="muted">{t('cap.antigravity.notInstalled')}</span>{@render installRow('antigravity')}
              {:else if on && st.antigravity.last_error}<Icon name="warning" size={13} /> <span class="msg">{explain('antigravity', st.antigravity.last_error, false)}</span>
              {:else if on}<Icon name="check" size={13} /> {t('cap.codex.ok', { t: ago(st.antigravity.last_ok_ms) })}
              {:else}{t('common.off')}{/if}
            {:else if kind === 'statusline'}
              {#if on}<Icon name="check" size={13} /> {t('cap.statusline.ok', { t: ago(st.statusline_last_ms) })}{#if st.statusline_chained} · {t('cap.statusline.chained')}{/if}
              {:else}{t('common.off')}{/if}
            {:else}
              {#if on && st.otel_listening}<Icon name="check" size={13} /> {t('cap.otel.ok', { port: st.otel_port, n: fmtInt(st.otel_events), t: ago(st.otel_last_ms) })}
              {:else if on}<Icon name="warning" size={13} /> <span class="msg">{st.otel_error ? explain('otel', st.otel_error, false) : t('cap.otel.notListening')}</span>
              {:else}{t('common.off')}{/if}
            {/if}
          </dd>
        </dl>
        {#if on && busy !== kind && signIn(kind) && (kind === 'claude' || kind === 'codex' || kind === 'antigravity')}
          <SignInSteps {kind} binary={st[kind].binary} />
        {/if}
        {#if note[kind]}
          <p class="note small" class:bad={!note[kind].ok}>{note[kind].text}</p>
        {/if}
        <p class="subtle small risk"><Icon name="info" size={12} /> {t(`cap.${kind}.note`)}</p>
      </div>
    {/each}
  {/if}
</section>

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .lead {
    margin: 6px 0 12px;
    max-width: 760px;
  }
  .method {
    border: 0.5px solid var(--hairline);
    border-radius: 12px;
    padding: 14px 16px;
    margin-top: 10px;
    background: var(--surface);
    transition: border-color var(--dur) var(--ease);
  }
  .method.on {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--hairline));
  }
  .top {
    display: flex;
    gap: 16px;
    align-items: center;
  }
  .txt {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  dl {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 4px 12px;
    margin: 12px 0 0;
    font-size: 12.5px;
  }
  dt {
    color: var(--ink-3);
  }
  dd {
    margin: 0;
    color: var(--ink-2);
    display: flex;
    align-items: center;
    gap: 5px;
    flex-wrap: wrap;
  }
  .install {
    flex-basis: 100%;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 6px;
  }
  .msg {
    flex: 1 1 0;
    min-width: 0;
  }
  code {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 11.5px;
    word-break: break-all;
  }
  .note {
    margin-top: 10px;
    color: var(--good-ink);
  }
  .note.bad {
    color: var(--bad-ink);
  }
  .risk {
    margin-top: 8px;
    display: flex;
    gap: 5px;
    align-items: flex-start;
  }
  .spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--hairline-strong);
    border-top-color: var(--accent);
    animation: sp 0.8s linear infinite;
  }
  @keyframes sp {
    to {
      transform: rotate(360deg);
    }
  }
</style>
