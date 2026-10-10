<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog'
  import { app, saveSettings } from '../lib/store.svelte'
  import { api, type ParserWarning, type SourceId, type SourceInfo } from '../lib/api'
  import { fmtDuration, t } from '../lib/i18n.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Icon from '../components/Icon.svelte'
  import CapturePanel from '../components/CapturePanel.svelte'

  let sources: SourceInfo[] = $state([])
  let warnings: ParserWarning[] = $state([])
  let now = $state(Date.now())

  let gen = 0
  async function load() {
    const mine = ++gen
    const [s, w] = await Promise.all([api.detectSources(), api.parserWarnings()])
    if (mine !== gen) return
    sources = s
    warnings = w
    now = Date.now()
  }
  $effect(() => {
    void app.tick
    void app.scan?.last_scan_ms
    load().catch(() => {})
    return () => gen++
  })

  async function setEnabled(id: SourceId, on: boolean) {
    await saveSettings((c) => {
      const cur = new Set(c.enabled_sources)
      if (on) cur.add(id)
      else cur.delete(id)
      return { enabled_sources: [...cur] }
    })
    load()
  }

  async function addFolder(kind: 'claude' | 'codex') {
    const dir = await open({ directory: true, multiple: false })
    if (typeof dir !== 'string') return
    await saveSettings((c) => {
      const ep = { ...c.extra_paths }
      if (kind === 'claude') ep.claude_config_dirs = [...new Set([...ep.claude_config_dirs, dir])]
      else ep.codex_homes = [...new Set([...ep.codex_homes, dir])]
      return { extra_paths: ep }
    })
    load()
  }
  async function removeFolder(kind: 'claude' | 'codex', dir: string) {
    await saveSettings((c) => {
      const ep = { ...c.extra_paths }
      if (kind === 'claude') ep.claude_config_dirs = ep.claude_config_dirs.filter((d) => d !== dir)
      else ep.codex_homes = ep.codex_homes.filter((d) => d !== dir)
      return { extra_paths: ep }
    })
    load()
  }
</script>

<header class="bar">
  <h1>{t('sources.title')}</h1>
  <span class="spacer"></span>
  {#if app.scan?.running}
    <span class="muted small">{t('sources.scanning', { done: app.scan.done, total: app.scan.total })}</span>
  {:else if app.scan?.last_scan_ms}
    <span class="muted small">{t('sources.lastScan', { t: fmtDuration(now - app.scan.last_scan_ms) })}</span>
  {/if}
  <button class="btn" onclick={() => api.rescan()} disabled={app.scan?.running}><Icon name="refresh" size={15} />{t('sources.rescan')}</button>
</header>
<p class="subtle small help"><Icon name="lock" size={13} /> {t('sources.help')}</p>

<section class="card list-card">
  {#each sources as s (s.id)}
    <div class="list-row src">
      <div class="state" class:found={s.found && !s.cloud_only} aria-hidden="true"><Icon name={s.cloud_only ? 'info' : s.found ? 'check' : 'close'} size={14} /></div>
      <div class="body">
        <div class="title">
          <b>{t(`source.${s.id}`)}</b>
          <span class="tag">{s.cloud_only ? t('source.cloud') : s.found ? (s.supported ? t('source.found') : t('source.unsupported')) : t('source.notFound')}</span>
          {#if s.file_count}<span class="subtle small">{t('source.files', { n: s.file_count })}</span>{/if}
        </div>
        <div class="subtle small">{t(s.cloud_only ? `source.desc.${s.id}.cloud` : `source.desc.${s.id}`)}</div>
        {#each s.roots as r (r)}<div class="path small">{r}</div>{/each}
      </div>
      {#if s.supported}
        <Toggle checked={s.enabled} label={t(`source.${s.id}`)} onchange={(v) => setEnabled(s.id, v)} />
      {/if}
    </div>
  {/each}
</section>

<section class="card">
  <h2>{t('sources.extra')}</h2>
  {#each [['claude', t('sources.extra.claude'), app.settings?.extra_paths.claude_config_dirs ?? []], ['codex', t('sources.extra.codex'), app.settings?.extra_paths.codex_homes ?? []]] as [kind, label, dirs] (kind)}
    <div class="extra">
      <div class="row">
        <span class="muted">{label}</span>
        <span class="spacer"></span>
        <button class="btn" onclick={() => addFolder(kind as 'claude' | 'codex')}><Icon name="plus" size={14} />{t('sources.addFolder')}</button>
      </div>
      {#each dirs as d (d)}
        <div class="list-row">
          <Icon name="folder" size={15} /><span class="path">{d}</span><span class="spacer"></span>
          <button class="btn ghost" aria-label={t('common.remove')} onclick={() => removeFolder(kind as 'claude' | 'codex', d as string)}><Icon name="trash" size={15} /></button>
        </div>
      {/each}
    </div>
  {/each}
</section>

{#if warnings.length}
  <section class="card">
    <h2><Icon name="warning" size={16} /> {t('sources.warnings')}</h2>
    <p class="subtle small help">{t('sources.warnings.help')}</p>
    {#each warnings.slice(0, 20) as w, i (i)}
      <div class="list-row warn">
        <span class="path small">{w.path}</span>
        <span class="spacer"></span>
        <span class="subtle small">{w.last} ({w.count})</span>
      </div>
    {/each}
  </section>
{/if}

<CapturePanel />

<div class="banner retention"><Icon name="clock" size={16} />{t('sources.retention')}</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 0 8px;
  }
  .spacer {
    flex: 1;
  }
  .help {
    display: flex;
    gap: 6px;
    margin-bottom: 14px;
    max-width: 760px;
  }
  section {
    margin-bottom: 16px;
  }
  .list-card {
    padding: 4px 20px;
  }
  .src {
    align-items: flex-start;
    padding: 14px 0;
  }
  .state {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--surface-hover);
    color: var(--ink-3);
    margin-top: 1px;
  }
  .state.found {
    color: var(--good-ink);
  }
  .body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .title {
    display: flex;
    gap: 8px;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .tag {
    font-size: 11.5px;
    color: var(--ink-2);
    background: var(--surface-hover);
    padding: 1px 7px;
    border-radius: 6px;
  }
  .path {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 11.5px;
    color: var(--ink-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .extra {
    margin-top: 12px;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .retention {
    margin-top: 16px;
  }
</style>
