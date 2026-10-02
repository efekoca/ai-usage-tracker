<script lang="ts">
  import { ask, open, save } from '@tauri-apps/plugin-dialog'
  import { onMount } from 'svelte'
  import { app, refresh, saveSettings } from '../lib/store.svelte'
  import { api, type PricingFile } from '../lib/api'
  import { fmtDate, fmtInt, t } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Icon from '../components/Icon.svelte'

  const s = $derived(app.settings!)
  let pricing: PricingFile | null = $state(null)
  let pricingOrigin = $state('bundled')
  let pricingDirty = $state(false)
  let notice = $state('')
  let models: string[] = $state([])
  let aliasFrom = $state('')
  let aliasTo = $state('')
  let exportFormat: 'csv' | 'json' = $state('csv')
  let exportGran: 'events' | 'daily' = $state('daily')

  async function loadPricing() {
    const p = await api.pricing()
    pricing = p.file
    pricingOrigin = p.origin
    pricingDirty = false
  }
  onMount(async () => {
    await loadPricing()
    models = await api.models()
  })
  const unpriced = $derived(models.filter((m) => pricing && !pricing.models.some((x) => x.id === m || x.aliases?.includes(m)) && !(m in (pricing.user_aliases ?? {}))))

  function flash(msg: string) {
    notice = msg
    setTimeout(() => (notice = ''), 4000)
  }
  async function guard(fn: () => Promise<void>) {
    try {
      await fn()
    } catch (e) {
      flash(t('common.error', { e: String(e) }))
    }
  }

  async function savePricing() {
    if (!pricing) return
    await guard(async () => {
      await api.savePricing($state.snapshot(pricing) as PricingFile)
      await loadPricing()
      await refresh()
      flash(t('settings.data.done'))
    })
  }
  async function resetPricing() {
    await guard(async () => {
      await api.resetPricing()
      await loadPricing()
      await refresh()
    })
  }
  function addAlias() {
    if (!pricing || !aliasFrom || !aliasTo) return
    pricing.user_aliases = { ...(pricing.user_aliases ?? {}), [aliasFrom]: aliasTo }
    pricingDirty = true
    aliasFrom = ''
  }
  function removeAlias(k: string) {
    if (!pricing) return
    const next = { ...pricing.user_aliases }
    delete next[k]
    pricing.user_aliases = next
    pricingDirty = true
  }
  function num(v: string): number | null {
    if (v.trim() === '') return null
    const n = Number(v.replace(',', '.'))
    return Number.isFinite(n) && n >= 0 ? n : null
  }

  async function doExport() {
    const ext = exportFormat
    const path = await save({ defaultPath: `ai-usage-${exportGran}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] })
    if (!path) return
    await guard(async () => {
      const n = await api.exportData(path, $state.snapshot(app.period), $state.snapshot(app.filter), exportGran, exportFormat)
      flash(t('settings.data.exported', { n: fmtInt(n) }))
    })
  }
  async function doBackup() {
    const stamp = new Date().toISOString().slice(0, 10)
    const path = await save({ defaultPath: `ai-usage-tracker-${stamp}.db`, filters: [{ name: 'SQLite', extensions: ['db'] }] })
    if (!path) return
    await guard(async () => {
      await api.backup(path)
      flash(t('settings.data.done'))
    })
  }
  async function doImport() {
    const path = await open({ multiple: false, filters: [{ name: 'SQLite', extensions: ['db', 'sqlite'] }] })
    if (typeof path !== 'string') return
    await guard(async () => {
      const r = await api.importDb(path)
      flash(t('settings.data.imported', { n: fmtInt(r.events) }))
    })
  }
  async function doWipe() {
    const ok = await ask(t('settings.data.wipe.confirm'), { title: t('settings.data.wipe'), kind: 'warning' })
    if (ok) await guard(() => api.wipe())
  }
</script>

<header class="bar">
  <h1>{t('settings.title')}</h1>
  {#if notice}<span class="notice" role="status"><Icon name="check" size={14} />{notice}</span>{/if}
</header>

<section class="card group">
  <h2>{t('settings.general')}</h2>
  <div class="item">
    <span>{t('settings.language')}</span>
    <select class="field" value={s.language} onchange={(e) => saveSettings({ language: e.currentTarget.value as 'system' | 'tr' | 'en' })}>
      <option value="system">{t('settings.language.system')}</option>
      <option value="tr">Türkçe</option>
      <option value="en">English</option>
    </select>
  </div>
  <div class="item">
    <span>{t('settings.theme')}</span>
    <Segmented label={t('settings.theme')} value={s.theme} options={[{ value: 'system', label: t('settings.theme.system') }, { value: 'light', label: t('settings.theme.light') }, { value: 'dark', label: t('settings.theme.dark') }]} onchange={(v) => saveSettings({ theme: v })} />
  </div>
  <div class="item">
    <span>{t('settings.primaryMetric')}</span>
    <Segmented label={t('settings.primaryMetric')} value={s.primary_metric} options={[{ value: 'tokens', label: t('metric.tokens') }, { value: 'cost', label: t('metric.cost') }]} onchange={(v) => saveSettings({ primary_metric: v })} />
  </div>
  <div class="item">
    <div><span>{t('limits.mode')}</span><div class="subtle small">{t('limits.mode.help')}</div></div>
    <Segmented label={t('limits.mode')} value={s.limit_display} options={[{ value: 'used', label: t('limits.mode.used') }, { value: 'remaining', label: t('limits.mode.remaining') }]} onchange={(v) => saveSettings({ limit_display: v })} />
  </div>
  <div class="item">
    <div><span>{t('settings.autostart')}</span><div class="subtle small">{t('settings.autostart.help')}</div></div>
    <Toggle checked={s.autostart} label={t('settings.autostart')} onchange={(v) => saveSettings({ autostart: v })} />
  </div>
  <div class="item">
    <div><span>{t('settings.currency')}</span><div class="subtle small">{t('settings.currency.help')}</div></div>
    <div class="row">
      <select class="field" value={s.currency} onchange={(e) => saveSettings({ currency: e.currentTarget.value })}>
        {#each ['USD', 'TRY', 'EUR', 'GBP'] as c (c)}<option value={c}>{c}</option>{/each}
      </select>
      {#if s.currency !== 'USD'}
        <label class="row small muted">{t('settings.fxRate')}
          <input class="field rate" inputmode="decimal" value={String(s.fx_rate)} onchange={(e) => { const n = num(e.currentTarget.value); if (n) saveSettings({ fx_rate: n }) }} />
          {s.currency}</label>
      {/if}
    </div>
  </div>
</section>

<section class="card group">
  <h2>{t('settings.widget')}</h2>
  <div class="item">
    <span>{t('settings.widget.show')}</span>
    <Toggle checked={s.widget.visible} label={t('settings.widget.show')} onchange={(v) => saveSettings({ widget: { ...s.widget, visible: v } })} />
  </div>
  <div class="item">
    <div><span>{t('ws.title')}</span><div class="subtle small">{t('settings.widget.studio')}</div></div>
    <button class="btn" onclick={() => (app.view = 'widget')}><Icon name="widget" size={15} />{t('ws.open')}</button>
  </div>
</section>

<section class="card group">
  <h2>{t('settings.privacy')}</h2>
  <div class="item">
    <span>{t('settings.hideProjects')}</span>
    <Toggle checked={s.hide_project_names} label={t('settings.hideProjects')} onchange={(v) => saveSettings({ hide_project_names: v }).then(refresh)} />
  </div>
  <p class="subtle small prose">{t('settings.privacy.text')}</p>
  <div class="item">
    <div><span>{t('settings.network')}</span><div class="subtle small">{t('settings.network.text')}</div></div>
    <Icon name="lock" size={16} />
  </div>
</section>

{#if pricing}
  <section class="card group">
    <div class="row">
      <h2>{t('settings.pricing')}</h2>
      <span class="spacer"></span>
      <span class="subtle small">{t(`settings.pricing.origin.${pricingOrigin}`)} · {t('settings.pricing.updated', { d: fmtDate(pricing.updated_at) })}</span>
    </div>
    <p class="subtle small prose">{t('settings.pricing.help')}</p>
    <div class="table-wrap">
      <table class="prices">
        <thead>
          <tr>
            <th scope="col">{t('common.model')}</th>
            <th scope="col" class="num">{t('metric.input')}</th>
            <th scope="col" class="num">{t('metric.output')}</th>
            <th scope="col" class="num">{t('metric.cacheRead')}</th>
            <th scope="col" class="num">{t('metric.cacheWrite')} 5m</th>
            <th scope="col" class="num">{t('metric.cacheWrite')} 1h</th>
          </tr>
        </thead>
        <tbody>
          {#each pricing.models as m (m.id)}
            <tr>
              <th scope="row" title={m.notes ?? ''}>{m.id}{#if m.long_context}<span class="subtle small"> · &gt;{fmtInt(m.long_context.threshold / 1000)}K</span>{/if}</th>
              {#each ['input', 'output', 'cache_read', 'cache_write_5m', 'cache_write_1h'] as const as k (k)}
                <td class="num">
                  <input
                    class="cell"
                    inputmode="decimal"
                    aria-label="{m.id} {k}"
                    value={m[k] ?? ''}
                    onchange={(e) => {
                      const v = num(e.currentTarget.value)
                      if (k === 'input' || k === 'output') {
                        if (v !== null) m[k] = v
                      } else m[k] = v
                      pricingDirty = true
                    }}
                  />
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <h3 class="sub">{t('settings.pricing.alias')}</h3>
    {#each Object.entries(pricing.user_aliases ?? {}) as [from, to] (from)}
      <div class="list-row">
        <code>{from}</code><span class="muted small">{t('settings.pricing.aliasAs')}</span><code>{to}</code>
        <span class="spacer"></span>
        <button class="btn ghost" aria-label={t('common.remove')} onclick={() => removeAlias(from)}><Icon name="trash" size={15} /></button>
      </div>
    {/each}
    <div class="row wrap">
      <select class="field" bind:value={aliasFrom} aria-label={t('common.model')}>
        <option value="">—</option>
        {#each unpriced as m (m)}<option value={m}>{m}</option>{/each}
      </select>
      <span class="muted small">{t('settings.pricing.aliasAs')}</span>
      <select class="field" bind:value={aliasTo} aria-label={t('common.model')}>
        <option value="">—</option>
        {#each pricing.models as m (m.id)}<option value={m.id}>{m.id}</option>{/each}
      </select>
      <button class="btn" onclick={addAlias} disabled={!aliasFrom || !aliasTo}><Icon name="plus" size={14} /></button>
    </div>
    {#if s.dismissed_unpriced?.length}
      <div class="list-row">
        <span class="muted small">{t('settings.pricing.dismissed', { models: s.dismissed_unpriced.join(', ') })}</span>
        <span class="spacer"></span>
        <button class="btn ghost small" onclick={() => saveSettings({ dismissed_unpriced: [] })}>{t('settings.pricing.dismissedShow')}</button>
      </div>
    {/if}

    <div class="row wrap actions">
      {#each Object.entries(pricing.sources ?? {}) as [prov, url] (prov)}
        <button class="btn ghost small" onclick={() => api.openUrl(url)}><Icon name="external" size={13} />{prov === 'openai' ? 'OpenAI' : 'Anthropic'}</button>
      {/each}
      <span class="spacer"></span>
      <button class="btn" onclick={resetPricing} disabled={pricingOrigin === 'bundled' && !pricingDirty}>{t('settings.pricing.reset')}</button>
      <button class="btn primary" onclick={savePricing} disabled={!pricingDirty}>{t('settings.pricing.save')}</button>
    </div>
  </section>
{/if}

<section class="card group">
  <h2>{t('settings.data')}</h2>
  <div class="item">
    <span>{t('settings.data.export')}</span>
    <div class="row">
      <Segmented label={t('settings.data.export')} bind:value={exportGran} options={[{ value: 'daily', label: t('settings.data.exportDaily') }, { value: 'events', label: t('settings.data.exportEvents') }]} />
      <Segmented label="format" bind:value={exportFormat} options={[{ value: 'csv', label: 'CSV' }, { value: 'json', label: 'JSON' }]} />
      <button class="btn" onclick={doExport}><Icon name="download" size={15} />{t('settings.data.export')}</button>
    </div>
  </div>
  <div class="item">
    <span>{t('settings.data.backup')}</span>
    <button class="btn" onclick={doBackup}><Icon name="download" size={15} />{t('settings.data.backup')}</button>
  </div>
  <div class="item">
    <div><span>{t('settings.data.import')}</span><div class="subtle small">{t('settings.data.import.help')}</div></div>
    <button class="btn" onclick={doImport}><Icon name="upload" size={15} />{t('settings.data.import')}</button>
  </div>
  <div class="item">
    <div><span>{t('settings.data.openFolder')}</span><div class="subtle small path">{app.info?.data_dir}</div></div>
    <button class="btn" onclick={() => api.openDataFolder()}><Icon name="folder" size={15} />{t('settings.data.openFolder')}</button>
  </div>
  <div class="item">
    <span class="danger-text">{t('settings.data.wipe')}</span>
    <button class="btn danger" onclick={doWipe}><Icon name="trash" size={15} />{t('settings.data.wipe')}</button>
  </div>
</section>

<section class="card group">
  <h2>{t('settings.about')}</h2>
  <div class="item">
    <span>{t('app.name')} · {t('settings.version', { v: app.info?.version ?? '' })}</span>
    <button class="btn" onclick={() => api.quit()}><Icon name="power" size={15} />{t('settings.quit')}</button>
  </div>
</section>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 16px 0 14px;
  }
  .notice {
    display: inline-flex;
    gap: 6px;
    align-items: center;
    font-size: 13px;
    color: var(--ink-2);
    background: var(--surface);
    border: 0.5px solid var(--hairline);
    padding: 4px 10px;
    border-radius: 8px;
  }
  .group {
    margin-bottom: 16px;
    max-width: 900px;
  }
  .group h2 {
    margin-bottom: 6px;
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
  .prose {
    max-width: 720px;
    margin: 6px 0 8px;
  }
  .rate {
    width: 90px;
  }
  .spacer {
    flex: 1;
  }
  .wrap {
    flex-wrap: wrap;
  }
  .table-wrap {
    overflow-x: auto;
    margin: 8px 0;
  }
  .prices {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  .prices th,
  .prices td {
    padding: 4px 8px;
    border-bottom: 0.5px solid var(--hairline);
    text-align: left;
    white-space: nowrap;
  }
  .prices thead th {
    color: var(--ink-2);
    font-weight: 500;
  }
  .prices tbody th {
    font-weight: 500;
  }
  .num {
    text-align: right !important;
  }
  .cell {
    width: 72px;
    text-align: right;
    border: 0.5px solid transparent;
    background: transparent;
    border-radius: 6px;
    padding: 3px 6px;
    font-variant-numeric: tabular-nums;
  }
  .cell:hover,
  .cell:focus {
    border-color: var(--hairline-strong);
    background: var(--surface-2);
  }
  .sub {
    margin: 16px 0 6px;
  }
  code {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 12px;
  }
  .actions {
    margin-top: 14px;
  }
  .path {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
  }
  .danger-text {
    color: var(--bad-ink);
  }
</style>
