<script lang="ts">
  import { open as openDialog, save } from '@tauri-apps/plugin-dialog'
  import { app, saveSettings } from '../lib/store.svelte'
  import { api } from '../lib/api'
  import { fmtDate, t } from '../lib/i18n.svelte'
  import Segmented from './Segmented.svelte'
  import Toggle from './Toggle.svelte'
  import Icon from './Icon.svelte'

  type Which = 'lastWeek' | 'thisWeek' | 'last7'
  let which = $state<Which>('lastWeek')
  let busy = $state(false)
  let status = $state<{ ok: boolean; text: string } | null>(null)

  // local calendar dates, never UTC
  const ymd = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
  const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n)
  const range = $derived.by(() => {
    const today = new Date()
    const monday = addDays(today, -((today.getDay() + 6) % 7))
    if (which === 'lastWeek') return { from: ymd(addDays(monday, -7)), to: ymd(addDays(monday, -1)) }
    if (which === 'thisWeek') return { from: ymd(monday), to: ymd(today) }
    return { from: ymd(addDays(today, -6)), to: ymd(today) }
  })

  async function run() {
    status = null
    const path = await save({ defaultPath: `AI-Usage_${range.from}_${range.to}.pdf`, filters: [{ name: 'PDF', extensions: ['pdf'] }] })
    if (!path) return
    busy = true
    try {
      await api.exportReport(range.from, range.to, path)
      status = { ok: true, text: t('report.saved') }
    } catch (e) {
      const code = String(e)
      status = { ok: false, text: t('report.failed', { e: code }) }
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
    <p class="status small" class:bad={!status.ok} role="status">
      <Icon name={status.ok ? 'check' : 'warning'} size={13} />
      {status.text}
      {#if status.ok}<button class="btn ghost small" onclick={() => api.openLastReport()}>{t('report.open')}</button>{/if}
    </p>
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
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 6px;
    color: var(--good-ink);
  }
  .status.bad {
    color: var(--critical-ink, var(--critical));
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
