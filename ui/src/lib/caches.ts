import type { LiveCache } from './api'
import { clientLabel, has, t } from './i18n.svelte'
import { app, saveSettings } from './store.svelte'

export type CacheTone = 'ok' | 'soon' | 'urgent'

/** Why a refresh could not be asked for, in words. */
export function refreshError(e: unknown): string {
  const code = String(e)
  return has(`cache.live.error.${code}`) ? t(`cache.live.error.${code}`) : t('cache.live.error.other', { why: code })
}

/** "1:36": minutes and seconds left, rounded up so 0:00 means it ran out. */
export function mmss(ms: number): string {
  const s = Math.max(0, Math.ceil(ms / 1000))
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

/** Yellow in the last 2 minutes of a 5-minute cache or the last 10 of an hour's, red in the last 1 or 5. */
export function cacheTone(c: Pick<LiveCache, 'ttl_ms'>, leftMs: number): CacheTone {
  const short = c.ttl_ms <= 300_000
  if (leftMs < (short ? 60_000 : 300_000)) return 'urgent'
  if (leftMs < (short ? 120_000 : 600_000)) return 'soon'
  return 'ok'
}

export const cacheName = (c: Pick<LiveCache, 'project'>) => c.project || t('projects.hidden')

type Place = 'cache_widget' | 'cache_menubar'

/** Whether the widget or the menu bar shows the cache time: the same switch as in Settings. */
export const cacheShown = (place: Place) => !!app.settings?.cache_timer && !!app.settings[place]

/** Turned on from the Widget or Menu bar page while the feature is off, it shows the time there alone. */
export function setCacheShown(place: Place, on: boolean) {
  const s = app.settings
  if (!s) return
  if (!on) return saveSettings({ [place]: false })
  if (s.cache_timer) return saveSettings({ [place]: true })
  return saveSettings({ cache_timer: true, cache_widget: place === 'cache_widget', cache_menubar: place === 'cache_menubar', notify: { ...s.notify, cache: false } })
}

/** Where a conversation runs: "Terminal", "Claude desktop" and so on. */
export function cacheClient(client: string | null): string {
  if (!client) return ''
  return has(`cache.client.${client}`) ? t(`cache.client.${client}`) : clientLabel(client, client)
}

/** "Opus 5.5" for claude-opus-5-5; any other model as it is logged. */
export function shortModel(id: string): string {
  const m = /^claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?(?!\d)/.exec(id)
  return m ? `${m[1][0].toUpperCase()}${m[1].slice(1)} ${m[2]}${m[3] ? `.${m[3]}` : ''}` : id
}
