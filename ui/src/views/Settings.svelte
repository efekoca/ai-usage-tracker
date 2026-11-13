<script lang="ts">
  import { ask, open } from '@tauri-apps/plugin-dialog'
  import { saveAs } from '../lib/dialogs'
  import { onMount } from 'svelte'
  import { app, refresh, saveSettings } from '../lib/store.svelte'
  import { api, providersOf, type HotkeyStatus, type ModelPrice, type NotifySettings, type PricingFile, type PricingInfo, type ProjectChoice, type Provider } from '../lib/api'
  import { fmtDate, fmtDateTime, fmtDuration, fmtInput, fmtInt, fmtPct, localDate, parseNumber, t, windowLabel } from '../lib/i18n.svelte'
  import Segmented from '../components/Segmented.svelte'
  import Select from '../components/Select.svelte'
  import Toggle from '../components/Toggle.svelte'
  import Icon from '../components/Icon.svelte'
  import BrandIcon from '../components/BrandIcon.svelte'
  import ColorPick, { type Swatch } from '../components/ColorPick.svelte'
  import ReportCard from '../components/ReportCard.svelte'
  import { ensureRefresh, loadRefreshInfo, refresh as hook, removeRefresh } from '../lib/refresh.svelte'

  const s = $derived(app.settings!)

  // brand-like hues, then neutral ones; warnings keep their own colors whatever is picked
  const TOOL_SWATCHES: Swatch[] = [['#d97757', 'orange'], ['#10a37f', 'green'], ['#4285f4', 'blue'], ['#8b5cf6', 'purple'], ['#e87ba4', 'pink'], ['#52514e', 'gray']]
  function setTint(p: Provider, color: string) {
    saveSettings((c) => {
      const next = { ...c.provider_colors }
      if (color) next[p] = color
      else delete next[p]
      return { provider_colors: next }
    })
  }
  // "Start with Windows" only on Windows; elsewhere it starts at login
  const fx = $derived(s.currency === 'USD' ? 1 : s.fx_rate)
  // another currency waits for its own rate, so amounts never show at the old currency's rate
  let newCurrency = $state('')
  function pickCurrency(c: string) {
    newCurrency = c === s.currency || c === 'USD' ? '' : c
    if (c === 'USD' && s.currency !== 'USD') saveSettings({ currency: c })
  }
  function setRate(el: HTMLInputElement) {
    const n = num(el.value)
    if (!n) {
      el.value = newCurrency ? '' : fmtInput(s.fx_rate)
      return
    }
    saveSettings(newCurrency ? { currency: newCurrency, fx_rate: n } : { fx_rate: n })
    newCurrency = ''
  }
  function setBudget(el: HTMLInputElement) {
    const n = el.value.trim() ? num(el.value) : 0
    // a budget of 0 is off, and the field then shows "Off" as for an empty one
    if (n === 0) el.value = ''
    if (n !== null) saveSettings({ monthly_budget_usd: n / fx })
    else el.value = s.monthly_budget_usd > 0 ? fmtInput(Math.round(s.monthly_budget_usd * fx * 100) / 100) : ''
  }
  const saveNotify = (patch: Partial<NotifySettings>) => saveSettings((c) => ({ notify: { ...c.notify, ...patch } }))
  const autostartKey = $derived(app.info && app.info.platform !== 'windows' ? 'settings.autostart.mac' : 'settings.autostart')
  let pricing: PricingFile | null = $state(null)
  let pricingOrigin = $state('bundled')
  let pricingDirty = $state(false)
  let priceBase: PricingInfo['base'] = $state([])
  let priceStatus: PricingInfo['status'] | null = $state(null)
  let canUpdatePrices = $state(false)
  let checkingPrices = $state(false)
  let priceCheck = $state('')
  let notice = $state('')
  const SOURCE_NAMES: Record<string, string> = { anthropic: 'Anthropic', openai: 'OpenAI', google: 'Google', vertex_ai: 'Vertex AI', deepseek: 'DeepSeek', moonshotai: 'Moonshot AI', zai: 'Z.ai', alibaba: 'Alibaba Cloud', xai: 'xAI', mistral: 'Mistral', minimax: 'MiniMax' }
  let models: string[] = $state([])
  let cataloged: ModelPrice[] = $state([])
  let aliasFrom = $state('')
  let aliasTo = $state('')
  let exportFormat: 'csv' | 'csv_semicolon' | 'json' = $state('csv')
  let exportGran: 'events' | 'daily' = $state('daily')

  async function loadPricing() {
    const p = await api.pricing()
    pricing = p.file
    pricingOrigin = p.origin
    priceBase = p.base
    priceStatus = p.status
    canUpdatePrices = p.can_update
    pricingDirty = false
  }
  // the projects to follow caches in, loaded once the list is shown
  let cacheChoices: ProjectChoice[] | null = $state(null)
  $effect(() => {
    if (s.cache_timer && s.cache_projects === 'picked' && !cacheChoices) api.cacheProjectChoices().then((c) => (cacheChoices = c)).catch(() => (cacheChoices = []))
  })
  const follow = (path: string, on: boolean) => saveSettings({ cache_picked: on ? [...s.cache_picked, path] : s.cache_picked.filter((p) => p !== path) })
  $effect(() => {
    if (s.cache_timer) loadRefreshInfo()
  })
  // automatic refresh needs the hook, which is added only once the user agrees
  async function setAuto(on: boolean) {
    if (on && !(await ensureRefresh())) return
    saveSettings({ cache_auto: on })
  }

  let loadFailed = $state('')
  onMount(async () => {
    hk = await api.hotkeyStatus().catch(() => null)
    try {
      await loadPricing()
      models = await api.models()
    } catch (e) {
      loadFailed = String(e)
      return
    }
    cataloged = await api.catalogPrices(models).catch(() => [])
  })

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
  // by physical key (e.code), so the keyboard layout does not matter
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
      hkError = needsModifier()
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
      hkError = String(e) === 'hotkey_needs_modifier' ? needsModifier() : t(String(e) === 'hotkey_unknown_key' ? 'settings.hotkey.err.unknown' : 'settings.hotkey.err.taken')
    }
  }
  const MAC_KEYS: Record<string, string> = { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Super: '⌘', Up: '↑', Down: '↓', Left: '←', Right: '→' }
  const MAC_ORDER = ['Ctrl', 'Alt', 'Shift', 'Super']
  // macOS writes modifiers as symbols, in the order ⌃ ⌥ ⇧ ⌘
  const keycaps = (v: string) => {
    const parts = v.split('+')
    // Linux desktops name the key Super
    if (app.info?.platform === 'windows') return parts.map((k) => (k === 'Super' ? 'Win' : k))
    if (app.info?.platform !== 'macos') return parts
    const rank = (k: string) => (MAC_ORDER.includes(k) ? MAC_ORDER.indexOf(k) : MAC_ORDER.length)
    return [...parts].sort((a, b) => rank(a) - rank(b)).map((k) => MAC_KEYS[k] ?? k)
  }
  const needsModifier = () =>
    t(
      app.info?.platform === 'macos'
        ? 'settings.hotkey.err.hotkey_needs_modifier.mac'
        : app.info?.platform === 'windows'
          ? 'settings.hotkey.err.hotkey_needs_modifier'
          : 'settings.hotkey.err.hotkey_needs_modifier.linux',
    )

  let testing = $state(false)
  let testResult = $state<'shown' | 'denied' | 'failed' | null>(null)
  async function sendTest() {
    testing = true
    testResult = null
    try {
      testResult = await api.testNotification()
    } catch {
      testResult = 'failed'
    } finally {
      testing = false
    }
  }

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
    try {
      await api.installUpdate()
    } catch (e) {
      flash(String(e) === 'update_install_manual' ? t('settings.updates.manual') : t('common.error', { e: String(e) }))
    }
  }
  const unpriced = $derived(models.filter((m) => pricing && !pricing.models.some((x) => x.id === m || x.aliases?.includes(m)) && !(m in (pricing.user_aliases ?? {}))))
  // the full list is long; the models found in the logs are the ones worth editing
  let allPrices = $state(false)
  const used = (p: ModelPrice) => models.some((m) => m === p.id || p.aliases?.includes(m) || pricing?.user_aliases?.[m] === p.id)
  const usedPrices = $derived.by(() => pricing?.models.filter(used) ?? [])
  const shownPrices = $derived.by(() => (allPrices || !usedPrices.length ? (pricing?.models ?? []) : usedPrices))

  let noticeTimer: ReturnType<typeof setTimeout> | undefined
  function flash(msg: string) {
    clearTimeout(noticeTimer)
    notice = msg
    noticeTimer = setTimeout(() => (notice = ''), 4000)
  }
  async function guard(fn: () => Promise<void>) {
    try {
      await fn()
    } catch (e) {
      const key = `settings.data.err.${String(e)}`
      flash(t(key) === key ? t('common.error', { e: String(e) }) : t(key))
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
  async function checkPrices() {
    checkingPrices = true
    priceCheck = ''
    try {
      const changed = await api.checkPrices()
      priceCheck = t(changed ? 'settings.pricing.checkUpdated' : 'settings.pricing.checkUpToDate')
      if (changed) await refresh()
    } catch {
      // the status line below gives the reason
    } finally {
      checkingPrices = false
      // a fetched list replaces the base, and unsaved edits would be lost by reloading over them
      if (!pricingDirty) await loadPricing()
      else priceStatus = (await api.pricing()).status
    }
  }
  const RATE_KEYS = ['input', 'output', 'cache_read', 'cache_write_5m', 'cache_write_1h'] as const
  const baseOf = (m: ModelPrice) => priceBase.find((b) => b.id.toLowerCase() === m.id.toLowerCase())
  const changed = (m: ModelPrice) => {
    const b = baseOf(m)
    return !!b && RATE_KEYS.some((k) => (m[k] ?? null) !== (b[k] ?? null))
  }
  function revert(m: ModelPrice) {
    const b = baseOf(m)
    if (!b) return
    for (const k of RATE_KEYS) m[k] = b[k] as number
    pricingDirty = true
  }
  function removeOwn(id: string) {
    if (!pricing) return
    pricing.models = pricing.models.filter((m) => m.id !== id)
    pricingDirty = true
  }
  // a price typed in for a model no list has
  let own = $state({ model: '', input: '', output: '', cache_read: '', cache_write_5m: '' })
  let ownError = $state('')
  function addOwn() {
    if (!pricing || !own.model) return
    const [input, output] = [num(own.input), num(own.output)]
    if (input === null || output === null) {
      ownError = t('settings.pricing.ownNeed')
      return
    }
    const opt = (v: string) => (v.trim() === '' ? null : num(v))
    pricing.models = [...pricing.models, { id: own.model, provider: 'custom', aliases: [], input, output, cache_read: opt(own.cache_read), cache_write_5m: opt(own.cache_write_5m), cache_write_1h: null, verified_at: null, notes: null }]
    pricingDirty = true
    own = { model: '', input: '', output: '', cache_read: '', cache_write_5m: '' }
    ownError = ''
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
  const priceLabel = $derived({
    input: t('metric.input'),
    output: t('metric.output'),
    cache_read: t('metric.cacheRead'),
    cache_write_5m: t('cache.write5m'),
    cache_write_1h: t('cache.write1h'),
  })
  function num(v: string): number | null {
    const n = parseNumber(v)
    return n !== null && n >= 0 ? n : null
  }

  async function doExport() {
    const ext = exportFormat === 'json' ? 'json' : 'csv'
    const path = await saveAs(`ai-usage-${exportGran}.${ext}`, ext.toUpperCase(), ext)
    if (!path) return
    await guard(async () => {
      const n = await api.exportData(path, $state.snapshot(app.period), $state.snapshot(app.filter), exportGran, exportFormat)
      flash(t('settings.data.exported', { n: fmtInt(n) }))
    })
  }
  async function doBackup() {
    const stamp = localDate()
    const path = await saveAs(`ai-usage-tracker-${stamp}.db`, 'SQLite', 'db')
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
{#if loadFailed}
  <div class="banner failed" role="alert"><Icon name="warning" size={16} />{t('common.error', { e: loadFailed })}</div>
{/if}

<section class="card group" id="settings-general">
  <h2>{t('settings.general')}</h2>
  <div class="item">
    <span>{t('settings.language')}</span>
    <Select label={t('settings.language')} value={s.language} minWidth={170} options={[{ value: 'system', label: t('settings.language.system') }, { value: 'tr', label: 'Türkçe' }, { value: 'en', label: 'English' }]} onchange={(v) => saveSettings({ language: v as 'system' | 'tr' | 'en' })} />
  </div>
  <div class="item">
    <span>{t('settings.theme')}</span>
    <Segmented label={t('settings.theme')} value={s.theme} options={[{ value: 'system', label: t('settings.theme.system') }, { value: 'light', label: t('settings.theme.light') }, { value: 'dark', label: t('settings.theme.dark') }]} onchange={(v) => saveSettings({ theme: v })} />
  </div>
  {#if app.info?.accent_color}
    <div class="item">
      <div><span>{t('settings.accent')}</span><div class="subtle small">{t('settings.accent.help')}</div></div>
      <Segmented label={t('settings.accent')} value={s.accent} options={[{ value: 'app', label: t('settings.accent.app') }, { value: 'system', label: t('settings.accent.system') }]} onchange={(v) => saveSettings({ accent: v })} />
    </div>
  {/if}
  <div class="item">
    <div><span>{t('settings.toolColors')}</span><div class="subtle small">{t('settings.toolColors.help')}</div></div>
    <div class="tints">
      {#each providersOf(s.enabled_sources) as p (p)}
        <div class="tint">
          <span class="brandname tname"><BrandIcon provider={p} color={s.tint_icons ? s.provider_colors?.[p] || null : null} />{t(`provider.${p}`)}</span>
          <ColorPick value={s.provider_colors?.[p] ?? ''} label={t(`provider.${p}`)} defaultLabel={t('settings.toolColors.accent')} customLabel={t('ws.accent.custom')} swatches={TOOL_SWATCHES} onchange={(v) => setTint(p, v)} />
        </div>
      {/each}
    </div>
  </div>
  <div class="item">
    <div><span>{t('settings.showIcons')}</span><div class="subtle small">{t('settings.showIcons.help')}</div></div>
    <Toggle checked={s.show_icons} label={t('settings.showIcons')} onchange={(v) => saveSettings({ show_icons: v })} />
  </div>
  <div class="item">
    <div><span>{t('settings.tintIcons')}</span><div class="subtle small">{t('settings.tintIcons.help')}</div></div>
    <Toggle checked={s.tint_icons} label={t('settings.tintIcons')} onchange={(v) => saveSettings({ tint_icons: v })} />
  </div>
  <div class="item">
    <span>{t('settings.primaryMetric')}</span>
    <Segmented label={t('settings.primaryMetric')} value={s.primary_metric} options={[{ value: 'tokens', label: t('metric.tokens') }, { value: 'cost', label: t('metric.cost') }]} onchange={(v) => saveSettings({ primary_metric: v })} />
  </div>
  <div class="item">
    <div><span>{t('limits.mode')}</span><div class="subtle small">{t('limits.mode.help')}</div></div>
    <Segmented label={t('limits.mode')} value={s.limit_display} options={[{ value: 'used', label: t('limits.mode.used') }, { value: 'remaining', label: t('limits.mode.remaining') }]} onchange={(v) => saveSettings({ limit_display: v })} />
  </div>
  <div class="bundle">
    <div class="item">
      <div><span>{t('settings.router')}</span><div class="subtle small">{t('settings.router.help')}</div></div>
      <Toggle checked={s.account_router} label={t('settings.router')} onchange={(v) => saveSettings({ account_router: v })} />
    </div>
    {#if s.account_router}
      <div class="choices">
        <div class="choice">
          <span class="glyph"><Icon name="limits" size={16} /></span>
          <span class="label">{t('settings.router.page')}</span>
          <Toggle small checked={s.router_page} label="{t('settings.router')}: {t('settings.router.page')}" onchange={(v) => saveSettings({ router_page: v })} />
        </div>
        <div class="choice">
          <span class="glyph"><Icon name="bell" size={16} /></span>
          <span class="label">{t('settings.router.notify')}{#if !s.notify.enabled}<span class="subtle small">{t('settings.notifyOff')}</span>{/if}</span>
          <Toggle small checked={s.notify.route} disabled={!s.notify.enabled} label="{t('settings.router')}: {t('settings.router.notify')}" onchange={(v) => saveNotify({ route: v })} />
        </div>
      </div>
    {/if}
  </div>
  <div class="bundle">
    <div class="item">
      <div><span>{t('settings.cacheTimer')}</span><div class="subtle small">{t('settings.cacheTimer.help')}</div></div>
      <Toggle checked={s.cache_timer} label={t('settings.cacheTimer')} onchange={(v) => saveSettings({ cache_timer: v })} />
    </div>
    {#if s.cache_timer}
      <span class="choices-title">{t('settings.cacheTimer.where')}</span>
      <div class="choices">
        <div class="choice">
          <span class="glyph"><Icon name="widget" size={16} /></span>
          <span class="label">{t('settings.cacheTimer.widget')}</span>
          <Toggle small checked={s.cache_widget} label="{t('settings.cacheTimer')}: {t('settings.cacheTimer.widget')}" onchange={(v) => saveSettings({ cache_widget: v })} />
        </div>
        <div class="choice">
          <span class="glyph"><Icon name="menubar" size={16} /></span>
          <span class="label">{t('settings.cacheTimer.menubar')}<span class="subtle small">{t('settings.cacheTimer.menubar.help')}</span></span>
          <Toggle small checked={s.cache_menubar} label="{t('settings.cacheTimer')}: {t('settings.cacheTimer.menubar')}" onchange={(v) => saveSettings({ cache_menubar: v })} />
        </div>
        <div class="choice">
          <span class="glyph"><Icon name="terminal" size={16} /></span>
          <span class="label">{t('settings.cacheTimer.statusline')}<span class="subtle small">{s.capture.statusline ? t('settings.cacheTimer.statusline.help') : t('settings.cacheTimer.statusline.off')}</span></span>
          {#if !s.capture.statusline}<button class="btn ghost small" onclick={() => (app.view = 'sources')}>{t('settings.cacheTimer.statusline.open')}</button>{/if}
          <Toggle small checked={s.cache_statusline && s.capture.statusline} disabled={!s.capture.statusline} label="{t('settings.cacheTimer')}: {t('settings.cacheTimer.statusline')}" onchange={(v) => saveSettings({ cache_statusline: v })} />
        </div>
        <div class="choice">
          <span class="glyph"><Icon name="bell" size={16} /></span>
          <span class="label">{t('settings.cacheTimer.notify')}<span class="subtle small">{s.notify.enabled ? t('settings.cacheTimer.notify.help') : t('settings.notifyOff')}</span></span>
          <Toggle small checked={s.notify.cache} disabled={!s.notify.enabled} label="{t('settings.cacheTimer')}: {t('settings.cacheTimer.notify')}" onchange={(v) => saveNotify({ cache: v })} />
        </div>
      </div>
      <span class="choices-title">{t('settings.cacheTimer.projects')}</span>
      <div class="choices">
        <div class="choice plain">
          <span class="label">{t('settings.cacheTimer.followed')}</span>
          <Segmented label={t('settings.cacheTimer.followed')} value={s.cache_projects} options={[{ value: 'all', label: t('settings.cacheTimer.all') }, { value: 'picked', label: t('settings.cacheTimer.picked') }]} onchange={(v) => saveSettings({ cache_projects: v })} />
        </div>
        {#if s.cache_projects === 'picked' && cacheChoices}
          {#each cacheChoices as p (p.path)}
            {@const name = p.hidden ? `${t('projects.hidden')} #${p.id}` : p.name}
            <div class="choice plain">
              <span class="label" title={p.hidden ? undefined : p.path}>{name}</span>
              <Toggle small checked={s.cache_picked.includes(p.path)} label={t('settings.cacheTimer.follow', { name })} onchange={(v) => follow(p.path, v)} />
            </div>
          {:else}
            <div class="choice plain"><span class="label subtle small">{t('settings.cacheTimer.noProjects')}</span></div>
          {/each}
        {/if}
      </div>
      <span class="choices-title">{t('settings.cacheTimer.refresh')}</span>
      <div class="choices">
        <div class="choice plain">
          <span class="label">{t('settings.cacheTimer.auto')}<span class="subtle small">{t('settings.cacheTimer.auto.help')}</span></span>
          <Toggle small checked={s.cache_auto && !!hook.info?.installed} label={t('settings.cacheTimer.auto')} onchange={setAuto} />
        </div>
        {#if s.cache_auto && hook.info?.installed}
          <div class="choice plain">
            <span class="label">{t('settings.cacheTimer.lead')}</span>
            <Segmented label={t('settings.cacheTimer.lead')} value={String(s.cache_lead)} options={['1', '2', '5'].map((n) => ({ value: n, label: t('settings.cacheTimer.lead.n', { n }) }))} onchange={(v) => saveSettings({ cache_lead: Number(v) as 1 | 2 | 5 })} />
          </div>
        {/if}
      </div>
      <p class="refresh-note subtle small">
        {t('settings.cacheTimer.limit')}
        <span class="hook">
          {#if hook.info?.installed}
            <span title={hook.info.settings_file}>{t('settings.cacheTimer.hook')}</span>
            <button class="btn ghost small" onclick={() => removeRefresh().catch(() => {})}>{t('settings.cacheTimer.hook.remove')}</button>
          {:else if hook.info}
            <span>{t('settings.cacheTimer.hook.none')}</span>
            <button class="btn ghost small" onclick={() => ensureRefresh()}>{t('settings.cacheTimer.hook.add')}</button>
          {/if}
        </span>
      </p>
    {/if}
  </div>
  <div class="item">
    <div><span>{t(autostartKey)}</span><div class="subtle small">{t('settings.autostart.help')}</div></div>
    <Toggle checked={s.autostart} label={t(autostartKey)} onchange={(v) => saveSettings({ autostart: v })} />
  </div>
  {#if app.info?.platform === 'macos'}
    <div class="item">
      <div><span>{t('settings.dock')}</span><div class="subtle small">{t('settings.dock.help')}</div></div>
      <Toggle checked={s.show_in_dock} label={t('settings.dock')} onchange={(v) => saveSettings({ show_in_dock: v })} />
    </div>
  {/if}
  <div class="item">
    <div>
      <span>{t('settings.currency')}</span>
      <div class="subtle small">{t('settings.currency.help')}</div>
      {#if newCurrency}<div class="small err" role="alert">{t('settings.fxRate.needed', { c: newCurrency, old: s.currency })}</div>{/if}
    </div>
    <div class="row">
      <Select label={t('settings.currency')} value={newCurrency || s.currency} minWidth={110} options={['USD', 'TRY', 'EUR', 'GBP'].map((c) => ({ value: c, label: c }))} onchange={pickCurrency} />
      {#if (newCurrency || s.currency) !== 'USD'}
        <label class="row small muted">{t('settings.fxRate')}
          <input class="field rate" inputmode="decimal" value={newCurrency ? '' : fmtInput(s.fx_rate)} aria-invalid={!!newCurrency || undefined} onchange={(e) => setRate(e.currentTarget)} />
          {newCurrency || s.currency}</label>
      {/if}
    </div>
  </div>
  <div class="item">
    <div><span>{t('settings.budget')}</span><div class="subtle small">{t('settings.budget.help')}</div></div>
    <label class="row small muted">
      <input
        class="field rate"
        inputmode="decimal"
        aria-label={t('settings.budget')}
        placeholder={t('settings.budget.off')}
        value={s.monthly_budget_usd > 0 ? fmtInput(Math.round(s.monthly_budget_usd * fx * 100) / 100) : ''}
        onchange={(e) => setBudget(e.currentTarget)}
      />
      {s.currency}</label>
  </div>
</section>

<section class="card group" id="settings-notify">
  <h2>{t('settings.notify.title')}</h2>
  <div class="item">
    <div><span>{t('settings.notify')}</span><div class="subtle small">{t('settings.notify.help')}</div></div>
    <Toggle checked={s.notify.enabled} label={t('settings.notify')} onchange={(v) => saveNotify({ enabled: v })} />
  </div>
  {#if s.notify.enabled}
    <div class="item">
      <div><span>{t('settings.notify.early')}</span><div class="subtle small">{t('settings.notify.early.help')}</div></div>
      <Segmented
        label={t('settings.notify.early')}
        value={s.notify.early}
        options={[
          { value: 'warn', label: `${t('ws.warn')} ${fmtPct(s.widget.warn_at)}` },
          { value: 'high', label: `${t('ws.high')} ${fmtPct(s.widget.high_at)}` },
          { value: 'off', label: t('settings.notify.off') },
        ]}
        onchange={(v) => saveNotify({ early: v })}
      />
    </div>
    <div class="item">
      <span>{t('settings.notify.full')}</span>
      <Toggle checked={s.notify.full} label={t('settings.notify.full')} onchange={(v) => saveNotify({ full: v })} />
    </div>
    <div class="item">
      <span>{t('settings.notify.reset')}</span>
      <Toggle checked={s.notify.reset} label={t('settings.notify.reset')} onchange={(v) => saveNotify({ reset: v })} />
    </div>
    <div class="item">
      <span>{t('settings.notify.fiveHour')}</span>
      <Toggle checked={s.notify.five_hour} label={t('settings.notify.fiveHour')} onchange={(v) => saveNotify({ five_hour: v })} />
    </div>
    <div class="item">
      <span>{t('settings.notify.weekly')}</span>
      <Toggle checked={s.notify.seven_day} label={t('settings.notify.weekly')} onchange={(v) => saveNotify({ seven_day: v })} />
    </div>
    {#if s.monthly_budget_usd > 0}
      <div class="item">
        <span>{t('settings.notify.budget')}</span>
        <Toggle checked={s.notify.budget} label={t('settings.notify.budget')} onchange={(v) => saveNotify({ budget: v })} />
      </div>
    {/if}
    {#if s.limit_resume}
      <div class="item">
        <span>{t('settings.notify.resume')}</span>
        <Toggle checked={s.notify.resume} label={t('settings.notify.resume')} onchange={(v) => saveNotify({ resume: v })} />
      </div>
    {/if}
    <div class="item">
      <div><span>{t('settings.notify.signIn')}</span><div class="subtle small">{t('settings.notify.signIn.help')}</div></div>
      <Toggle checked={s.notify.sign_in} label={t('settings.notify.signIn')} onchange={(v) => saveNotify({ sign_in: v })} />
    </div>
    <div class="item">
      <div>
        <span>{t('settings.notify.test')}</span>
        <div class="subtle small" aria-live="polite">
          {#if testResult === 'denied' && app.info?.platform === 'windows'}{t('settings.notify.test.deniedWindows')}{:else if testResult}{t(`settings.notify.test.${testResult}`)}{:else}{t('settings.notify.test.help')}{/if}
        </div>
      </div>
      <button class="btn" onclick={sendTest} disabled={testing}>{testing ? t('settings.notify.test.sending') : t('settings.notify.test.send')}</button>
    </div>
  {/if}
</section>

<section class="card group" id="settings-widget">
  <h2>{t('settings.widget')}</h2>
  <div class="item">
    <span>{t('settings.widget.show')}</span>
    <Toggle checked={s.widget.visible} label={t('settings.widget.show')} onchange={(v) => saveSettings((c) => ({ widget: { ...c.widget, visible: v } }))} />
  </div>
  <div class="item">
    <div><span>{t('ws.title')}</span><div class="subtle small">{t('settings.widget.studio')}</div></div>
    <button class="btn" onclick={() => (app.view = 'widget')}><Icon name="widget" size={15} />{t('ws.open')}</button>
  </div>
  <div class="item">
    <div>
      <span>{t('settings.hotkey')}</span>
      <div class="subtle small">{t('settings.hotkey.help')}</div>
      {#if hk?.desktop}<div class="subtle small">{t('settings.hotkey.desktop')}</div>{/if}
      {#if hkError || hk?.error}<div class="small err" role="alert">{hkError || t(hk?.error === 'hotkey_declined' ? 'settings.hotkey.err.declined' : hk?.error === 'hotkey_unknown_key' ? 'settings.hotkey.err.unknown' : 'settings.hotkey.err.taken')}</div>{/if}
    </div>
    <div class="row wrap end">
      {#if recording}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="recorder" tabindex="0" role="textbox" aria-label={t('settings.hotkey.record')} onkeydown={onRecordKey} onblur={() => (recording = false)} {@attach (el: HTMLElement) => el.focus()}>
          {t('settings.hotkey.record')}
        </div>
      {:else if s.widget.hotkey}
        <span class="keys"><span class="sr-only">{s.widget.hotkey}</span>{#each keycaps(s.widget.hotkey) as k, i (i)}<kbd aria-hidden="true">{k}</kbd>{/each}</span>
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

<section class="card group" id="settings-updates">
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

<section class="card group" id="settings-privacy">
  <h2>{t('settings.privacy')}</h2>
  <div class="item">
    <span>{t('settings.hideProjects')}</span>
    <Toggle checked={s.hide_project_names} label={t('settings.hideProjects')} onchange={(v) => saveSettings({ hide_project_names: v }).then(refresh)} />
  </div>
  <p class="subtle small prose">{t('settings.privacy.text', { dir: app.info?.data_dir ?? '' })}</p>
  <div class="item">
    <div><span>{t('settings.network')}</span><div class="subtle small">{t('settings.network.text')}</div></div>
    <Icon name="lock" size={16} />
  </div>
</section>

{#if pricing}
  <section class="card group" id="settings-pricing">
    <div class="row">
      <h2>{t('settings.pricing')}</h2>
      <span class="spacer"></span>
      <span class="subtle small">{t(`settings.pricing.origin.${pricingOrigin}`)} · {t('settings.pricing.updated', { d: fmtDate(pricing.updated_at) })}</span>
    </div>
    <p class="subtle small prose">{t('settings.pricing.help')}</p>
    {#if canUpdatePrices}
      <div class="item">
        <div>
          <span>{t('settings.pricing.follow')}</span>
          <div class="subtle small">{t('settings.pricing.follow.help')}</div>
          <div class="subtle small" aria-live="polite">
            {#if priceCheck}{priceCheck}{:else if priceStatus?.last_error}{t('settings.pricing.checkFailed', { e: priceStatus.last_error })}{:else if priceStatus?.last_check_ms}{t('settings.pricing.checked', { t: fmtDuration(Date.now() - priceStatus.last_check_ms) })}{:else}{t('settings.pricing.never')}{/if}
          </div>
        </div>
        <div class="row">
          <button class="btn small" onclick={checkPrices} disabled={checkingPrices}>{checkingPrices ? t('settings.pricing.checking') : t('settings.pricing.checkNow')}</button>
          <Toggle checked={s.price_updates} label={t('settings.pricing.follow')} onchange={(v) => saveSettings({ price_updates: v })} />
        </div>
      </div>
    {/if}
    <div class="table-wrap">
      <table class="prices">
        <thead>
          <tr>
            <th scope="col">{t('common.model')}</th>
            <th scope="col" class="num">{t('metric.input')}</th>
            <th scope="col" class="num">{t('metric.output')}</th>
            <th scope="col" class="num">{t('metric.cacheRead')}</th>
            <th scope="col" class="num">{t('cache.write5m')}</th>
            <th scope="col" class="num">{t('cache.write1h')}</th>
            <th scope="col"><span class="sr-only">{t('settings.pricing.rowActions')}</span></th>
          </tr>
        </thead>
        <tbody>
          {#each shownPrices as m (m.id)}
            <tr>
              <th scope="row" title={m.notes ?? ''}>
                {m.id}{#if m.long_context}<span class="subtle small">{' · '}&gt;{fmtInt(m.long_context.threshold / 1000)}K</span>{/if}
                {#if !baseOf(m)}<span class="mark mine">{t('settings.pricing.mine')}</span>{:else if changed(m)}<span class="mark">{t('settings.pricing.changed')}</span>{/if}
              </th>
              {#each ['input', 'output', 'cache_read', 'cache_write_5m', 'cache_write_1h'] as const as k (k)}
                <td class="num">
                  <input
                    class="cell"
                    inputmode="decimal"
                    aria-label="{m.id}: {priceLabel[k]}"
                    value={fmtInput(m[k])}
                    onchange={(e) => {
                      const raw = e.currentTarget.value
                      const v = num(raw)
                      // text that is not a price goes back to the saved value instead of staying in the cell
                      if (v === null && (raw.trim() !== '' || k === 'input' || k === 'output')) {
                        e.currentTarget.value = fmtInput(m[k])
                        return
                      }
                      m[k] = v as number
                      pricingDirty = true
                    }}
                  />
                </td>
              {/each}
              <td class="act">
                {#if !baseOf(m)}
                  <button class="btn ghost icon" aria-label="{t('common.remove')}: {m.id}" title={t('common.remove')} onclick={() => removeOwn(m.id)}><Icon name="trash" size={14} /></button>
                {:else if changed(m)}
                  <button class="btn ghost icon" aria-label="{t('settings.pricing.revert')}: {m.id}" title={t('settings.pricing.revert')} onclick={() => revert(m)}><Icon name="refresh" size={14} /></button>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if cataloged.some((c) => !pricing?.models.some((m) => m.id === c.id))}
      <h3 class="sub">{t('settings.pricing.catalog')}</h3>
      <p class="subtle small prose">{t('settings.pricing.catalogHelp')}</p>
      <div class="list">
        {#each cataloged.filter((c) => !pricing?.models.some((m) => m.id === c.id)) as c (c.id)}
          <div class="list-row">
            <code>{c.id}</code>
            <span class="badge-unverified">{t('settings.pricing.unverified')}</span>
            <span class="muted small num">{t('settings.pricing.catalogRates', { i: fmtInput(c.input), o: fmtInput(c.output), r: c.cache_read == null ? '—' : fmtInput(c.cache_read) })}</span>
            <span class="spacer"></span>
            <button
              class="btn small"
              onclick={() => {
                if (!pricing) return
                pricing.models = [...pricing.models, { ...c, notes: null, verified_at: null }]
                pricingDirty = true
              }}>{t('settings.pricing.catalogAdd')}</button
            >
          </div>
        {/each}
      </div>
    {/if}
    {#if usedPrices.length && usedPrices.length < pricing.models.length}
      <div class="row">
        <span class="subtle small">{allPrices ? t('settings.pricing.allShown', { n: pricing.models.length }) : t('settings.pricing.usedShown', { n: usedPrices.length })}</span>
        <span class="spacer"></span>
        <button class="btn ghost small" aria-expanded={allPrices} onclick={() => (allPrices = !allPrices)}>
          {allPrices ? t('settings.pricing.showUsed') : t('settings.pricing.showAll', { n: pricing.models.length })}
        </button>
      </div>
    {/if}

    {#if unpriced.length}
      <h3 class="sub">{t('settings.pricing.own')}</h3>
      <div class="row wrap own">
        <Select label={t('common.model')} bind:value={own.model} minWidth={190} searchable placeholder="—" options={unpriced.map((m) => ({ value: m, label: m }))} />
        {#each [['input', t('metric.input')], ['output', t('metric.output')], ['cache_read', t('metric.cacheRead')], ['cache_write_5m', t('cache.write5m')]] as const as [k, label] (k)}
          <label class="own-field">
            <span class="subtle small">{label}</span>
            <input class="field" inputmode="decimal" placeholder={k === 'input' || k === 'output' ? '$' : '—'} bind:value={own[k]} oninput={() => (ownError = '')} />
          </label>
        {/each}
        <button class="btn" onclick={addOwn} disabled={!own.model}><Icon name="plus" size={14} />{t('settings.pricing.ownAdd')}</button>
      </div>
      <p class="subtle small prose">{ownError || t('settings.pricing.ownHelp')}</p>
    {/if}

    <h3 class="sub">{t('settings.pricing.alias')}</h3>
    {#each Object.entries(pricing.user_aliases ?? {}) as [from, to] (from)}
      <div class="list-row">
        <code>{from}</code><span class="muted small">{t('settings.pricing.aliasAs')}</span><code>{to}</code>
        <span class="spacer"></span>
        <button class="btn ghost" aria-label="{t('common.remove')}: {from}" onclick={() => removeAlias(from)}><Icon name="trash" size={15} /></button>
      </div>
    {/each}
    <div class="row wrap">
      <Select label={t('settings.pricing.aliasFrom')} bind:value={aliasFrom} minWidth={190} searchable placeholder="—" options={unpriced.map((m) => ({ value: m, label: m }))} />
      <span class="muted small">{t('settings.pricing.aliasAs')}</span>
      <Select label={t('settings.pricing.aliasTo')} bind:value={aliasTo} minWidth={190} searchable placeholder="—" options={pricing.models.map((m) => ({ value: m.id, label: m.id }))} />
      <button class="btn" aria-label={t('settings.pricing.aliasAdd')} title={t('settings.pricing.aliasAdd')} onclick={addAlias} disabled={!aliasFrom || !aliasTo}><Icon name="plus" size={14} /></button>
    </div>
    {#if s.dismissed_unpriced?.length}
      <div class="list-row">
        <span class="muted small">{t('settings.pricing.dismissed', { models: s.dismissed_unpriced.join(', ') })}</span>
        <span class="spacer"></span>
        <button class="btn ghost small" onclick={() => saveSettings({ dismissed_unpriced: [] })}>{t('settings.pricing.dismissedShow')}</button>
      </div>
    {/if}

    <div class="row wrap sources">
      <span class="muted small">{t('settings.pricing.sources')}</span>
      {#each Object.entries(pricing.sources ?? {}) as [prov, url] (prov)}
        <button class="btn ghost small" onclick={() => api.openUrl(url)}><Icon name="external" size={13} />{SOURCE_NAMES[prov] ?? prov}</button>
      {/each}
    </div>
    <div class="row wrap actions">
      <span class="spacer"></span>
      <button class="btn" onclick={resetPricing} disabled={pricingOrigin !== 'user' && !pricingDirty}>{t('settings.pricing.reset')}</button>
      <button class="btn primary" onclick={savePricing} disabled={!pricingDirty}>{t('settings.pricing.save')}</button>
    </div>
  </section>
{/if}

<ReportCard />

<section class="card group" id="settings-data">
  <h2>{t('settings.data')}</h2>
  <div class="item">
    <div><span>{t('settings.data.export')}</span>{#if exportFormat === 'csv_semicolon'}<div class="subtle small">{t('settings.data.csvSemicolon.help')}</div>{/if}</div>
    <div class="row">
      <Segmented label={t('settings.data.export')} bind:value={exportGran} options={[{ value: 'daily', label: t('settings.data.exportDaily') }, { value: 'events', label: t('settings.data.exportEvents') }]} />
      <Segmented label={t('settings.data.format')} bind:value={exportFormat} options={[{ value: 'csv', label: 'CSV' }, { value: 'csv_semicolon', label: t('settings.data.csvSemicolon') }, { value: 'json', label: 'JSON' }]} />
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

<section class="card group" id="settings-about">
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
  .group,
  .failed {
    margin-bottom: 16px;
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
  /* a switch and the places it shows, in a panel under it */
  .bundle {
    border-bottom: 0.5px solid var(--hairline);
  }
  .bundle > .item {
    border-bottom: 0;
  }
  .choices {
    margin: -2px 0 14px;
    background: var(--surface-2);
    border: 0.5px solid var(--hairline);
    border-radius: 12px;
    overflow: hidden;
  }
  .choice {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
  }
  .choice + .choice::before {
    content: '';
    position: absolute;
    top: 0;
    left: 54px;
    right: 0;
    height: 0.5px;
    background: var(--hairline);
  }
  .glyph {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background: var(--surface-hover);
    color: var(--ink-2);
  }
  .choice.plain::before {
    left: 14px;
  }
  .refresh-note {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: -6px 2px 14px;
    line-height: 1.5;
  }
  .hook {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .choices-title {
    display: block;
    margin: 2px 2px 6px;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--ink-3);
  }
  .choice .label {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 13px;
  }
  .tints {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .tint {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .tname {
    min-width: 110px;
  }
  .prose {
    max-width: 720px;
    margin: 6px 0 8px;
  }
  section[id] {
    scroll-margin-top: 12px;
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
  .sources {
    gap: 2px 4px;
    margin-top: 14px;
  }
  .mark {
    margin-left: 6px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-press);
    color: var(--ink-2);
    font-size: 10.5px;
    font-weight: 500;
  }
  .mark.mine {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .act {
    width: 30px;
    padding: 0 !important;
  }
  .icon {
    width: 26px;
    height: 26px;
    padding: 0;
    justify-content: center;
  }
  .own {
    align-items: flex-end;
    gap: 8px;
  }
  .own-field {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .own-field .field {
    width: 84px;
  }
  .badge-unverified {
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--warning) 18%, transparent);
    color: var(--ink-2);
    font-size: 11px;
    font-weight: 600;
  }
</style>
