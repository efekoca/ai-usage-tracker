<script lang="ts">
  import { ask, open, save } from '@tauri-apps/plugin-dialog'
  import { onMount } from 'svelte'
  import { app, refresh, saveSettings } from '../lib/store.svelte'
  import { api, type HotkeyStatus, type PricingFile } from '../lib/api'
  import { fmtDate, fmtDateTime, fmtInt, fmtPct, t, windowLabel } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import Select from '../components/Select.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Icon from '../components/Icon.svelte'
  import ReportCard from '../components/ReportCard.svelte'

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
    hk = await api.hotkeyStatus().catch(() => null)
    await loadPricing()
    models = await api.models()
  })

  // ---- widget shortcut: press the combination to record it
  const DEFAULT_HOTKEY = 'Ctrl+Alt+Shift+W'
  let hk: HotkeyStatus | null = $state(null)
  let recording = $state(false)
  let hkError = $state('')
  const NAMED: Record<string, string> = {
    Space: 'Space', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right', Home: 'Home', End: 'End',
    PageUp: 'PageUp', PageDown: 'PageDown', Insert: 'Insert', Delete: 'Delete', Backquote: 'Backquote', Minus: 'Minus',
    Equal: 'Equal', BracketLeft: 'BracketLeft', BracketRight: 'BracketRight', Backslash: 'Backslash', Semicolon: 'Semicolon',
    Quote: 'Quote', Comma: 'Comma', Period: 'Period', Slash: 'Slash', Pause: 'Pause', PrintScreen: 'PrintScreen',
  }
  /** The key as the shortcut parser names it (by physical position, so the layout does not matter). */
  function keyName(code: string): string | null {
    if (/^Key[A-Z]$/.test(code)) return code.slice(3)
    if (/^Digit\d$/.test(code)) return code.slice(5)
    if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code
    if (/^Numpad\d$/.test(code)) return code
    return NAMED[code] ?? null
  }
  function onRecordKey(e: KeyboardEvent) {
    e.preventDefault()
    e.stopPropagation()
    const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Super'].filter(Boolean) as string[]
    if (e.key === 'Escape' && mods.length === 0) {
      recording = false
      hkError = ''
      return
    }
    if (['Control', 'Alt', 'Shift', 'Meta', 'AltGraph'].includes(e.key)) return // wait for the main key
    const key = keyName(e.code)
    if (!key) return
    if (mods.length === 0) {
      hkError = t('settings.hotkey.err.hotkey_needs_modifier')
      return
    }
    setHotkey([...mods, key].join('+'))
  }
  async function setHotkey(v: string) {
    try {
      hk = await api.setHotkey(v)
      if (app.settings) app.settings = { ...app.settings, widget: { ...app.settings.widget, hotkey: hk.hotkey } }
      hkError = ''
      recording = false
      flash(t('settings.hotkey.saved'))
    } catch (e) {
      hkError = String(e) === 'hotkey_needs_modifier' ? t('settings.hotkey.err.hotkey_needs_modifier') : t('settings.hotkey.err.taken')
    }
  }
  const keycaps = (v: string) => v.split('+').map((k) => (k === 'Super' ? 'Win' : k))

  // ---- updates
  const upd = $derived(app.update)
  let checking = $state(false)
  async function checkNow() {
    checking = true
    try {
      app.update = await api.checkUpdate()
    } catch (e) {
      app.update = await api.updateStatus().catch(() => app.update)
    } finally {
      checking = false
    }
  }
  async function installNow(v: string) {
    if (!(await ask(t('settings.updates.confirm', { v }), { title: t('settings.updates'), kind: 'info' }))) return
    await guard(() => api.installUpdate())
  }
  const trayLimits = [
    ['anthropic', 'five_hour'],
    ['anthropic', 'seven_day'],
    ['openai', 'five_hour'],
    ['openai', 'seven_day'],
  ] as const
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
    <Select label={t('settings.language')} value={s.language} minWidth={170} options={[{ value: 'system', label: t('settings.language.system') }, { value: 'tr', label: 'Türkçe' }, { value: 'en', label: 'English' }]} onchange={(v) => saveSettings({ language: v as 'system' | 'tr' | 'en' })} />
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
      <Select label={t('settings.currency')} value={s.currency} minWidth={110} options={['USD', 'TRY', 'EUR', 'GBP'].map((c) => ({ value: c, label: c }))} onchange={(v) => saveSettings({ currency: v })} />
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
  <div class="item">
    <div>
      <span>{t('settings.hotkey')}</span>
      <div class="subtle small">{t('settings.hotkey.help')}</div>
      {#if hkError || hk?.error}<div class="small err" role="alert">{hkError || t('settings.hotkey.err.taken')}</div>{/if}
    </div>
    <div class="row wrap end">
      {#if recording}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="recorder" tabindex="0" role="textbox" aria-label={t('settings.hotkey.record')} onkeydown={onRecordKey} onblur={() => (recording = false)} {@attach (el: HTMLElement) => el.focus()}>
          {t('settings.hotkey.record')}
        </div>
      {:else if s.widget.hotkey}
        <span class="keys" aria-label={s.widget.hotkey}>{#each keycaps(s.widget.hotkey) as k, i (i)}<kbd>{k}</kbd>{/each}</span>
      {:else}
        <span class="subtle small">{t('settings.hotkey.none')}</span>
      {/if}
      {#if !recording}
        <button class="btn" onclick={() => { hkError = ''; recording = true }}><Icon name="keyboard" size={15} />{t('settings.hotkey.change')}</button>
        {#if s.widget.hotkey}<button class="btn ghost" onclick={() => setHotkey('')}>{t('settings.hotkey.clear')}</button>{/if}
        {#if s.widget.hotkey !== DEFAULT_HOTKEY}<button class="btn ghost" onclick={() => setHotkey(DEFAULT_HOTKEY)}>{t('settings.hotkey.reset')}</button>{/if}
      {/if}
    </div>
  </div>
</section>

<section class="card group">
  <h2>{t('settings.tray')}</h2>
  <p class="subtle small prose">{t('settings.tray.help')}</p>
  <div class="item">
    <div><span>{t('settings.tray.show')}</span><div class="subtle small">{t('settings.tray.showHelp', { warn: fmtPct(s.widget.warn_at), high: fmtPct(s.widget.high_at) })}</div></div>
    <Toggle checked={s.tray.show_percent} label={t('settings.tray.show')} onchange={(v) => saveSettings({ tray: { ...s.tray, show_percent: v } })} />
  </div>
  <div class="item">
    <span>{t('settings.tray.limit')}</span>
    <Select
      label={t('settings.tray.limit')}
      value={s.tray.limit}
      disabled={!s.tray.show_percent}
      minWidth={220}
      options={[{ value: 'auto', label: t('settings.tray.auto') }, ...trayLimits.map(([p, w]) => ({ value: `${p}:${w}`, label: `${t(`provider.${p}`)} · ${windowLabel(w)}` }))]}
      onchange={(v) => saveSettings({ tray: { ...s.tray, limit: v } })}
    />
  </div>
</section>

<section class="card group">
  <h2>{t('settings.updates')}</h2>
  {#if upd && !upd.configured}
    <p class="subtle small prose">{t('settings.updates.notConfigured')} {t('settings.updates.version', { v: upd.current })}</p>
  {:else if upd}
    <div class="item">
      <div><span>{t('settings.updates.auto')}</span><div class="subtle small">{t('settings.updates.autoHelp')}</div></div>
      <Toggle checked={s.update_check} label={t('settings.updates.auto')} onchange={(v) => saveSettings({ update_check: v })} />
    </div>
    <div class="item">
      <div>
        <span>{t('settings.updates.version', { v: upd.current })}</span>
        <div class="subtle small" role="status">
          {#if upd.installing}
            {t('settings.updates.installing', { pct: upd.total ? fmtPct((upd.downloaded / upd.total) * 100) : '…' })}
          {:else if upd.available}
            {t('settings.updates.available', { v: upd.available.version })}{upd.available.notes ? ` · ${upd.available.notes}` : ''}
          {:else if upd.last_error}
            {t('settings.updates.error', { e: upd.last_error })}
          {:else if upd.last_check_ms}
            {t('settings.updates.upToDate', { t: fmtDateTime(upd.last_check_ms) })}
          {:else}
            {t('settings.updates.never')}
          {/if}
        </div>
        {#if upd.installing && upd.total}
          <div class="progress" aria-hidden="true"><span style="width:{(upd.downloaded / upd.total) * 100}%"></span></div>
        {/if}
      </div>
      <div class="row">
        {#if upd.available && !upd.installing}
          <button class="btn primary" onclick={() => installNow(upd.available!.version)}><Icon name="download" size={15} />{t('settings.updates.install')}</button>
        {/if}
        <button class="btn" onclick={checkNow} disabled={checking || upd.checking || upd.installing}><Icon name="refresh" size={15} />{checking || upd.checking ? t('settings.updates.checking') : t('settings.updates.check')}</button>
      </div>
    </div>
  {/if}
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
      <Select label={t('common.model')} bind:value={aliasFrom} minWidth={190} searchable placeholder="—" options={unpriced.map((m) => ({ value: m, label: m }))} />
      <span class="muted small">{t('settings.pricing.aliasAs')}</span>
      <Select label={t('common.model')} bind:value={aliasTo} minWidth={190} searchable placeholder="—" options={pricing.models.map((m) => ({ value: m.id, label: m.id }))} />
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

<ReportCard />

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
  .end {
    justify-content: flex-end;
  }
  .err {
    color: var(--bad-ink);
    margin-top: 4px;
  }
  .keys {
    display: inline-flex;
    gap: 4px;
  }
  kbd {
    font: 600 12px/1 var(--font);
    padding: 5px 7px;
    border-radius: 6px;
    background: var(--surface-2);
    border: 0.5px solid var(--hairline-strong);
    box-shadow: 0 1px 0 var(--hairline-strong);
  }
  .recorder {
    font-size: 13px;
    padding: 7px 12px;
    border-radius: 8px;
    border: 1px dashed var(--accent);
    color: var(--ink-2);
    outline: none;
  }
  .progress {
    height: 4px;
    border-radius: 2px;
    background: var(--surface-hover);
    margin-top: 6px;
    overflow: hidden;
    max-width: 320px;
  }
  .progress span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
</style>
