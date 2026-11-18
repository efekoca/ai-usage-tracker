import type { PanelData, PanelLimit } from './api'

/** Antigravity's other pool is any pool that is not Gemini; a module without an account follows the account in use. */
export function ofModule(provider: string, pool: string, account: string) {
  return (l: PanelLimit) => l.provider === provider && (account ? l.account === account : l.active) && (provider !== 'google' || (pool === 'gemini') === (l.limit_id === 'gemini'))
}

const USUAL = ['five_hour', 'seven_day']
const ORDER = ['five_hour', 'one_day', 'seven_day', 'thirty_day']

/** A provider tile's two windows: 5-hour and weekly, or the plan's own when it has neither (Codex's free plan has one of 30 days). */
export function tileWindows(data: PanelData | null, provider: string, pool: string, account = ''): string[] {
  const present = [...new Set((data?.limits ?? []).filter(ofModule(provider, pool, account)).map((l) => l.window))]
  if (present.length === 0 || present.some((w) => USUAL.includes(w))) return USUAL
  const rank = (w: string) => (ORDER.includes(w) ? ORDER.indexOf(w) : ORDER.length)
  return present.sort((a, b) => rank(a) - rank(b)).slice(0, 2)
}
