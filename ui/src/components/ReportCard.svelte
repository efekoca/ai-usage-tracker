<script lang="ts">
  import { open as openDialog, save } from '@tauri-apps/plugin-dialog'
  import { app, saveSettings } from '../lib/store.svelte'
  import { api } from '../lib/api'
  import { fmtDate, has, localDate, t } from '../lib/i18n.svelte'
  import Segmented from './Segmented.svelte'
  import Toggle from './Toggle.svelte'
  import Icon from './Icon.svelte'

  type Which = 'lastWeek' | 'thisWeek' | 'last7'
  let which = $state<Which>('lastWeek')
  let busy = $state(false)
  let status = $state<{ ok: boolean; text: string } | null>(null)

  const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n)
  function rangeOf(w: Which, today = new Date()) {
    const monday = addDays(today, -((today.getDay() + 6) % 7))
    if (w === 'lastWeek') return { from: localDate(addDays(monday, -7)), to: localDate(addDays(monday, -1)) }
    if (w === 'thisWeek') return { from: localDate(monday), to: localDate(today) }
    return { from: localDate(addDays(today, -6)), to: localDate(today) }
  }
  const range = $derived(rangeOf(which))

  async function run() {
    status = null
    // the page may have stayed open past midnight: take the dates now, not when it was drawn
    const r = rangeOf(which)
    const path = await save({ defaultPath: `AI-Usage_${r.from}_${r.to}.pdf`, filters: [{ name: 'PDF', extensions: ['pdf'] }] })
    if (!path) return
    busy = true
    try {
      await api.exportReport(r.from, r.to, path)
      status = { ok: true, text: t('report.saved') }
    } catch (e) {
      const code = String(e)
      status = { ok: false, text: t('report.failed', { e: has(`report.err.${code}`) ? t(`report.err.${code}`) : code }) }
    } finally {
      busy = false
    }
  }
  async function chooseFolder() {
    const dir = await openDialog({ directory: true })
    if (typeof dir === 'string') saveSettings({ weekly_report_dir: dir })
  }
</script>

<section class="card group">
  <h2><Icon name="report" size={17} /> {t('report.title')}</h2>
  <p class="subtle small help">{t('report.help')}</p>
  <div class="item">
    <div class="row wrap">
      <Segmented
        label={t('report.title')}
        bind:value={which}
        options={[{ value: 'lastWeek', label: t('report.lastWeek') }, { value: 'thisWeek', label: t('report.thisWeek') }, { value: 'last7', label: t('report.last7') }]}
      />
      <span class="muted small">{fmtDate(range.from, 'medium')} – {fmtDate(range.to, 'medium')}</span>
    </div>
    <button class="btn primary" onclick={run} disabled={busy}>
      {#if busy}<span class="spin" aria-hidden="true"></span>{t('report.saving')}{:else}<Icon name="download" size={15} />{t('report.save')}{/if}
    </button>
  </div>
  {#if status}
    <div class="status small" class:bad={!status.ok} role={status.ok ? 'status' : 'alert'}>
      <Icon name={status.ok ? 'check' : 'warning'} size={15} />
      <span class="msg">{status.text}</span>
      {#if status.ok}<button class="btn ghost small" onclick={() => api.openLastReport()}>{t('report.open')}</button>{/if}
    </div>
  {/if}
  <div class="item">
    <div><span>{t('report.auto')}</span><div class="subtle small">{t('report.autoHelp')}</div></div>
    <Toggle checked={!!app.settings?.weekly_report_auto} label={t('report.auto')} onchange={(v) => saveSettings({ weekly_report_auto: v })} />
  </div>
  {#if app.settings?.weekly_report_auto}
    <div class="item">
      <div class="folder"><span>{t('report.folder')}</span><div class="subtle small path">{app.settings.weekly_report_dir || app.info?.reports_dir}</div></div>
      <button class="btn" onclick={chooseFolder}><Icon name="folder" size={15} />{t('report.choose')}</button>
    </div>
  {/if}
</section>

<style>
  .group {
    margin-bottom: 16px;
    max-width: 900px;
  }
  .item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 0;
    border-bottom: 0.5px solid var(--hairline);
  }
  .item:last-child {
    border-bottom: 0;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .help {
    margin: 4px 0 6px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .wrap {
    flex-wrap: wrap;
  }
  .status {
    --tone: var(--good-ink);
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 12px 0;
    padding: 10px 12px;
    border-radius: 10px;
    border: 0.5px solid color-mix(in srgb, var(--tone) 35%, var(--hairline));
    background: color-mix(in srgb, var(--tone) 9%, transparent);
    color: var(--ink-2);
    line-height: 1.45;
  }
  .status.bad {
    --tone: var(--critical);
  }
  .status > :global(svg) {
    flex: none;
    color: var(--tone);
  }
  .status .msg {
    flex: 1;
    min-width: 0;
  }
  .folder {
    min-width: 0;
  }
  .path {
    word-break: break-all;
  }
  .spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-right-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
