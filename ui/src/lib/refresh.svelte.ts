import { api, type RefreshInfo } from './api'

/** Whether the refresh hook is in Claude Code's settings, and the consent dialog's state. */
export const refresh = $state<{ info: RefreshInfo | null; asking: boolean; busy: boolean; error: string }>({ info: null, asking: false, busy: false, error: '' })

let answer: ((ok: boolean) => void) | null = null

export async function loadRefreshInfo() {
  try {
    refresh.info = await api.cacheRefreshInfo()
  } catch {
    /* backend busy; the next look retries */
  }
}

/** The hook is added only after the user agrees in the dialog; true once it is there. */
export function ensureRefresh(): Promise<boolean> {
  if (refresh.info?.installed) return Promise.resolve(true)
  refresh.error = ''
  refresh.asking = true
  return new Promise((done) => (answer = done))
}

export async function answerConsent(ok: boolean) {
  if (ok) {
    refresh.busy = true
    try {
      refresh.info = await api.setCacheRefresh(true)
    } catch (e) {
      // the dialog stays open and says why, e.g. a settings file that is not valid JSON
      refresh.error = String(e)
      return
    } finally {
      refresh.busy = false
    }
  }
  refresh.asking = false
  answer?.(ok && !!refresh.info?.installed)
  answer = null
}

export async function removeRefresh() {
  refresh.info = await api.setCacheRefresh(false)
}
