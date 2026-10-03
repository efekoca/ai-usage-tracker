// Dashboard state shared across views (Svelte 5 runes).
import { api, on, type AppInfo, type Filter, type LimitView, type Period, type Report, type ScanStatus, type Settings, type UpdateStatus } from './api'
import { i18n, resolveLang } from './i18n.svelte'

export type View = 'overview' | 'tips' | 'daily' | 'breakdown' | 'sessions' | 'cache' | 'context' | 'limits' | 'projects' | 'sources' | 'widget' | 'settings'

export const app = $state({
  ready: false,
  info: null as AppInfo | null,
  settings: null as Settings | null,
  view: 'overview' as View,
  period: { kind: 'days7' } as Period,
  filter: {} as Filter,
  report: null as Report | null,
  limits: [] as LimitView[],
  loading: false,
  error: '',
  scan: null as ScanStatus | null,
  update: null as UpdateStatus | null,
  /** Limits page tab, so other pages can open the history directly. */
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

export async function refresh() {
  if (!app.settings?.onboarded) return
  app.loading = true
  try {
    const [report, limits] = await Promise.all([api.report($state.snapshot(app.period), $state.snapshot(app.filter)), api.limits()])
    app.report = report
    app.limits = limits
    app.error = ''
  } catch (e) {
    app.error = String(e)
  } finally {
    app.loading = false
    app.tick++
  }
}

export function refreshSoon() {
  clearTimeout(refreshTimer)
  refreshTimer = setTimeout(refresh, 250)
}

export async function saveSettings(patch: Partial<Settings>) {
  if (!app.settings) return
  const next = { ...$state.snapshot(app.settings), ...patch } as Settings
  app.settings = await api.saveSettings(next)
  applyAppearance(app.settings, app.info)
}

export async function init() {
  const [info, settings] = await Promise.all([api.appInfo(), api.getSettings()])
  app.info = info
  app.settings = settings
  applyAppearance(settings, info)
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
  // the tray menu's "update" item opens the settings
  await on<View>('navigate', (v) => {
    app.view = v
  })
  app.update = await api.updateStatus().catch(() => null)
  // follow OS light/dark switches live
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => app.tick++)
  // limit states (reset/stale, countdowns) move with time
  setInterval(() => {
    if (app.settings?.onboarded) api.limits().then((l) => (app.limits = l)).catch(() => {})
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

/** Series color per tool — follows the entity, never its rank. */
export const toolColor: Record<string, string> = {
  codex: 'var(--s1)',
  claude_code: 'var(--s2)',
  claude_desktop: 'var(--s3)',
}
