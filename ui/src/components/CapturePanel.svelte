<script lang="ts">
  // Opt-in live capture switches with what each one changes, its live status and errors.
  import { onMount } from 'svelte'
  import { app } from '../lib/store.svelte'
  import { api, type CaptureStatus } from '../lib/api'
  import { fmtDuration, fmtInt, t } from '../lib/i18n.svelte'
  import Toggle from './Toggle.svelte'
  import Icon from './Icon.svelte'
  import AccuracyBadge from './AccuracyBadge.svelte'

  let st = $state<CaptureStatus | null>(null)
  let busy = $state<string | null>(null)
  let note = $state<Record<string, { ok: boolean; text: string }>>({})
  let now = $state(Date.now())

  async function load() {
    st = await api.captureStatus()
    now = Date.now()
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

  async function toggle(kind: 'codex' | 'statusline' | 'otel', on: boolean) {
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
  const ago = (ms: number | null | undefined) => (ms ? t('cap.ago', { t: fmtDuration(now - ms) }) : t('cap.never'))
</script>

<section class="card">
  <div class="head">
    <h2>{t('cap.title')}</h2>
    <AccuracyBadge kind="captured" />
  </div>
  <p class="subtle small lead">{t('cap.lead')}</p>

  {#if st}
    {#each [['codex', st.codex_poll], ['statusline', st.statusline], ['otel', st.otel]] as const as [kind, on] (kind)}
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
            {#if kind === 'codex'}{t('cap.codex.changes')}
            {:else}{t(`cap.${kind}.changes`)} <code>{st.settings_file}</code>{/if}
          </dd>
          <dt>{t('cap.status')}</dt>
          <dd>
            {#if busy === kind}
              <span class="spin" aria-hidden="true"></span> {t('cap.working')}
            {:else if kind === 'codex'}
              {#if !st.codex_candidates_found}<Icon name="warning" size={13} /> {t('cap.msg.codex_not_found')}
              {:else if on && st.codex.last_error}<Icon name="warning" size={13} /> {explain('codex', st.codex.last_error, false)}
              {:else if on}<Icon name="check" size={13} /> {t('cap.codex.ok', { t: ago(st.codex.last_ok_ms) })}
              {:else}{t('common.off')}{/if}
            {:else if kind === 'statusline'}
              {#if on}<Icon name="check" size={13} /> {t('cap.statusline.ok', { t: ago(st.statusline_last_ms) })}{#if st.statusline_chained} · {t('cap.statusline.chained')}{/if}
              {:else}{t('common.off')}{/if}
            {:else}
              {#if on && st.otel_listening}<Icon name="check" size={13} /> {t('cap.otel.ok', { port: st.otel_port, n: fmtInt(st.otel_events), t: ago(st.otel_last_ms) })}
              {:else if on}<Icon name="warning" size={13} /> {st.otel_error ?? t('cap.otel.notListening')}
              {:else}{t('common.off')}{/if}
            {/if}
          </dd>
        </dl>
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
