import { api, on, type Account, type AppInfo, type Filter, type LimitView, type Period, type Provider, type Report, type ScanStatus, type Settings, type UpdateStatus } from './api'
import { i18n, resolveLang, t } from './i18n.svelte'

const VIEWS = ['overview', 'tips', 'daily', 'breakdown', 'sessions', 'cache', 'context', 'limits', 'projects', 'sources', 'widget', 'menubar', 'settings'] as const
export type View = (typeof VIEWS)[number]

export const app = $state({
  ready: false,
  info: null as AppInfo | null,
  settings: null as Settings | null,
  view: 'overview' as View,
  period: { kind: 'days7' } as Period,
  filter: {} as Filter,
  report: null as Report | null,
  limits: [] as LimitView[],
  /** Codex accounts; empty while there is only one, and then nothing about accounts shows. */
  accounts: [] as Account[],
  loading: false,
  error: '',
  scan: null as ScanStatus | null,
  update: null as UpdateStatus | null,
  limitsTab: 'current' as 'current' | 'history',
  /** bumps whenever backend data changes, so views can refetch their own data */
  tick: 0,
})

export function applyAppearance(s: Settings, info: AppInfo | null) {
  const root = document.documentElement
  if (s.theme === 'system') delete root.dataset.theme
  else root.dataset.theme = s.theme
  root.dataset.mica = String(!!info?.supports_mica && !root.classList.contains('widget'))
  if (info?.accent_color) {
    root.style.setProperty('--accent', info.accent_color)
    root.style.setProperty('--accent-ink', inkOn(info.accent_color))
  }
  i18n.lang = resolveLang(s.language)
  i18n.currency = s.currency || 'USD'
  i18n.fx = s.fx_rate || 1
  root.lang = i18n.lang
}

/** Black or white text, whichever contrasts more with a hex background. */
function inkOn(hex: string): string {
  const n = parseInt(hex.slice(1), 16)
  const lin = (c: number) => {
    const v = c / 255
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
  }
  const L = 0.2126 * lin((n >> 16) & 255) + 0.7152 * lin((n >> 8) & 255) + 0.0722 * lin(n & 255)
  return (L + 0.05) / 0.05 > 1.05 / (L + 0.05) ? '#0b0b0b' : '#ffffff'
}

let refreshTimer: ReturnType<typeof setTimeout> | undefined
let refreshGen = 0

export async function refresh() {
  if (!app.settings?.onboarded) return
  const gen = ++refreshGen
  app.loading = true
  try {
    const [report, limits, accounts] = await Promise.all([api.report($state.snapshot(app.period), $state.snapshot(app.filter)), api.limits(), api.accounts()])
    if (gen !== refreshGen) return
    app.report = report
    app.limits = limits
    app.accounts = accounts
    app.error = ''
  } catch (e) {
    if (gen === refreshGen) app.error = String(e)
  } finally {
    if (gen === refreshGen) {
      app.loading = false
      app.tick++
    }
  }
}

export function refreshSoon() {
  clearTimeout(refreshTimer)
  refreshTimer = setTimeout(refresh, 250)
}

/** Runs `load` and passes its result on only while the caller still wants it; returns the cancel. */
export function latest<T>(load: () => Promise<T>, done: (v: T) => void, fail?: (e: unknown) => void): () => void {
  let live = true
  load().then(
    (v) => live && done(v),
    (e) => live && fail?.(e),
  )
  return () => {
    live = false
  }
}

export type SettingsPatch = Partial<Settings> | ((s: Settings) => Partial<Settings>)
let saving: Promise<unknown> = Promise.resolve()

/** One save at a time, each patch applied to the newest settings, so quick changes never undo each other. */
export function saveSettings(patch: SettingsPatch): Promise<void> {
  const run = async () => {
    if (!app.settings) return
    const cur = $state.snapshot(app.settings) as Settings
    const next = { ...cur, ...(typeof patch === 'function' ? patch(cur) : patch) } as Settings
    app.settings = await api.saveSettings(next)
    applyAppearance(app.settings, app.info)
  }
  const p = saving.then(run)
  saving = p.catch(() => {})
  return p
}

export async function init() {
  const [info, settings] = await Promise.all([api.appInfo(), api.getSettings()])
  app.info = info
  app.settings = settings
  applyAppearance(settings, info)
  // the tray panel opens a new window straight on a page
  const start = location.hash.slice(1)
  if ((VIEWS as readonly string[]).includes(start)) app.view = start as View
  app.scan = await api.scanStatus()
  app.ready = true

  await on('data-changed', () => refreshSoon())
  await on<Settings>('settings-changed', (s) => {
    const wasOnboarded = app.settings?.onboarded
    app.settings = s
    applyAppearance(s, app.info)
    if (!wasOnboarded && s.onboarded) refreshSoon()
  })
  await on<{ done: number; total: number }>('scan-progress', (p) => {
    app.scan = { ...(app.scan ?? ({} as ScanStatus)), running: true, done: p.done, total: p.total }
  })
  await on<ScanStatus>('scan-finished', (s) => {
    app.scan = s
  })
  await on<UpdateStatus>('update-status', (u) => {
    app.update = u
  })
  // the tray menu and panel open pages in a window that is already open
  await on<View>('navigate', (v) => {
    app.view = v
  })
  app.update = await api.updateStatus().catch(() => null)
  // follow OS light/dark switches live
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => app.tick++)
  // limit states (reset/stale, countdowns) move with time
  setInterval(() => {
    if (!app.settings?.onboarded) return
    const gen = refreshGen
    api.limits().then((l) => gen === refreshGen && (app.limits = l)).catch(() => {})
  }, 60_000)

  await refresh()
}

export function setPeriod(p: Period) {
  app.period = p
  refresh()
}

export function setFilter(f: Filter) {
  app.filter = f
  refresh()
}

/** The account a provider's limit or series belongs to, named; empty while there is one account. */
export function accountOf(v: { provider: Provider; account: string | null }): string {
  return app.accounts.length && v.provider === 'openai' ? accountName(v.account) : ''
}

/** The user's name for a Codex account, else its number in order of first use. */
export function accountName(id: string | null): string {
  if (id === null) return t('accounts.unknown')
  const own = app.settings?.account_names?.[id]
  if (own) return own
  const n = app.accounts.filter((a) => a.id !== null).findIndex((a) => a.id === id)
  return t('accounts.numbered', { n: n < 0 ? '?' : n + 1 })
}

/** Series color per tool — follows the entity, never its rank. */
export const toolColor: Record<string, string> = {
  codex: 'var(--s1)',
  claude_code: 'var(--s2)',
  claude_desktop: 'var(--s3)',
  antigravity: 'var(--s4)',
  opencode: 'var(--s5)',
}

/** The color the user picked for a tool's limits and mark, if any. */
export const tint = (p: Provider): string | null => app.settings?.provider_colors?.[p] || null
/** Marks stay in the text color unless the user also colors them. */
export const markTint = (p: Provider): string | null => (app.settings?.tint_icons ? tint(p) : null)

export const providerColor: Record<string, string> = {
  openai: 'var(--s1)',
  anthropic: 'var(--s2)',
  google: 'var(--s4)',
}
