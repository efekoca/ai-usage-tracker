import type { DayDetail, DayPoint, Group, Report, Sessions, Tokens } from './api'
import { fmtClock, fmtCompact, fmtDate, fmtDec, fmtHour, fmtInt, fmtMoney, fmtPct, i18n, t } from './i18n.svelte'

export type CardKind = 'day' | 'month' | 'year'

/** A piece of a line; `**bold**` in a string marks the bold pieces. */
export interface Run {
  text: string
  bold: boolean
}
export interface Stat {
  label: string
  value: string
  detail: string
}
export type Chart =
  | { kind: 'heat'; levels: number[]; offset: number; ticks: { col: number; label: string }[]; less: string; more: string }
  | { kind: 'bars'; values: number[]; avg: number | null; ticks: { i: number; label: string }[] }

/** Everything a summary card shows, in the app's language. */
export interface Card {
  eyebrow: string
  title: string
  squareTitle: string
  sub: string
  hero: string
  unit: string
  line: Run[][]
  chart: Chart
  left: Run[]
  right: Run[]
  avgLine: boolean
  stats: Stat[]
  square: Stat[]
  /** How much of the dial is filled: active days or hours out of those that have begun. */
  share: number
}

export function runs(s: string): Run[] {
  return s.split('**').map((text, i) => ({ text, bold: i % 2 === 1 })).filter((r) => r.text)
}

export function longestStreak(days: { events: number }[]): number {
  let best = 0
  let run = 0
  for (const d of days) {
    run = d.events > 0 ? run + 1 : 0
    best = Math.max(best, run)
  }
  return best
}

/** The month (0–11) with the most tokens. */
export function peakMonth(days: DayPoint[]): { month: number; tokens: number } | null {
  const sums = new Map<number, number>()
  for (const d of days) sums.set(Number(d.date.slice(5, 7)) - 1, (sums.get(Number(d.date.slice(5, 7)) - 1) ?? 0) + d.tokens)
  let best: { month: number; tokens: number } | null = null
  for (const [month, tokens] of sums) if (tokens > 0 && (!best || tokens > best.tokens)) best = { month, tokens }
  return best
}

/** 0 for a day without use, else 1–4 by quartile among the days with use, as contribution graphs do. */
export function heatLevels(values: number[]): number[] {
  const used = values.filter((v) => v > 0).sort((a, b) => a - b)
  const q = [0.25, 0.5, 0.75].map((p) => used[Math.floor(p * (used.length - 1))] ?? 0)
  return values.map((v) => (v > 0 ? 1 + q.filter((x) => v > x).length : 0))
}

/** Read share of prompt tokens, as the Cache page counts it. */
export function cacheHit(k: Tokens): number | null {
  const prompt = k.input + k.cache_read + k.cache_write
  return prompt > 0 ? (k.cache_read / prompt) * 100 : null
}

function topModel(groups: Group[], total: number): Stat | null {
  const g = [...groups].sort((a, b) => b.totals.total_tokens - a.totals.total_tokens)[0]
  if (!g || total <= 0) return null
  return { label: t('cards.stat.model'), value: g.label, detail: t('cards.detail.modelShare', { pct: fmtPct((g.totals.total_tokens / total) * 100) }) }
}

const locale = () => (i18n.lang === 'tr' ? 'tr-TR' : 'en-US')
const upper = (s: string) => s.toLocaleUpperCase(locale())
const monthName = (m: number, year = 2000) => new Intl.DateTimeFormat(locale(), { month: 'long' }).format(new Date(year, m, 1))
const dayMonth = (d: Date) => new Intl.DateTimeFormat(locale(), { day: 'numeric', month: 'long' }).format(d)
const multipleRun = (n: number | null) => (n !== null ? [runs(t('cards.multiple', { n: fmtDec(n, n < 10 ? 1 : 0) }))] : [])
const cacheStat = (k: Tokens): Stat => {
  const hit = cacheHit(k)
  return { label: t('cards.stat.cache'), value: hit !== null ? fmtPct(hit) : '—', detail: t('cards.detail.cache') }
}

/** `days` counts the year's days that have begun since the archive's first request. */
export function buildYear(r: Report, year: number, days: number, multiple: number | null): Card | null {
  if (r.totals.events === 0) return null
  const first = new Date(year, 0, 1)
  // Monday-first rows, as the Daily page's heatmap
  const offset = (first.getDay() + 6) % 7
  const ticks = [0, 3, 6, 9].map((m) => {
    const doy = Math.round((new Date(year, m, 1).getTime() - first.getTime()) / 86_400_000)
    return { col: Math.floor((doy + offset) / 7), label: new Intl.DateTimeFormat(locale(), { month: 'short' }).format(new Date(year, m, 1)) }
  })
  const pm = peakMonth(r.daily)
  const streak = longestStreak(r.daily)
  const peak = r.peak_day
  const active: Stat = {
    label: t('cards.stat.active'),
    value: `${fmtInt(r.active_days)} / ${fmtInt(days)}`,
    detail: t('cards.detail.daysShare', { pct: fmtPct((r.active_days / Math.max(1, days)) * 100) }),
  }
  const month: Stat | null = pm ? { label: t('cards.stat.peakMonth'), value: monthName(pm.month, year), detail: t('cards.detail.tokens', { v: fmtCompact(pm.tokens) }) } : null
  const streakStat: Stat = { label: t('cards.stat.streak'), value: t('days.streakValue', { n: streak }), detail: t('cards.detail.inARow') }
  const stats = [
    month,
    peak ? { label: t('cards.stat.peakDay'), value: dayMonth(new Date(peak.date + 'T00:00:00')), detail: t('cards.detail.tokensCost', { v: fmtCompact(peak.tokens), usd: fmtMoney(peak.cost_usd) }) } : null,
    streakStat,
    topModel(r.by_model, r.totals.total_tokens),
  ].filter((s): s is Stat => !!s)
  return {
    eyebrow: upper(t('cards.eyebrow.year')),
    title: String(year),
    squareTitle: String(year),
    sub: t('cards.sub.year'),
    hero: fmtCompact(r.totals.total_tokens),
    unit: t('cards.unit'),
    line: [runs(t('cards.apiEq', { v: fmtMoney(r.totals.cost_usd) })), ...multipleRun(multiple)],
    chart: { kind: 'heat', levels: heatLevels(r.daily.map((d) => d.tokens)), offset, ticks, less: t('cards.less'), more: t('cards.more') },
    left: [],
    right: runs(t('cards.activeDays', { n: fmtInt(r.active_days) })),
    avgLine: false,
    stats,
    square: [active, month, streakStat].filter((s): s is Stat => !!s),
    share: days > 0 ? r.active_days / days : 0,
  }
}

/** `days` counts the month's days that have begun since the archive's first request. */
export function buildMonth(r: Report, s: Sessions, start: Date, days: number, multiple: number | null): Card | null {
  if (r.totals.events === 0) return null
  const values = r.daily.map((d) => d.tokens)
  const usedDays = r.daily.filter((d) => d.events > 0)
  const avg = usedDays.length ? usedDays.reduce((a, d) => a + d.tokens, 0) / usedDays.length : null
  const peak = r.peak_day
  const active: Stat = {
    label: t('cards.stat.active'),
    value: `${fmtInt(r.active_days)} / ${fmtInt(days)}`,
    detail: t('cards.detail.streak', { n: longestStreak(r.daily) }),
  }
  const sessions: Stat = {
    label: t('cards.stat.sessions'),
    value: fmtInt(s.sessions.length),
    detail: t('cards.detail.perDay', { v: fmtDec(s.sessions.length / Math.max(1, r.active_days), 1) }),
  }
  const cache = cacheStat(r.totals.tokens)
  return {
    eyebrow: upper(t('cards.eyebrow.month')),
    title: monthName(start.getMonth(), start.getFullYear()),
    squareTitle: fmtDate(start, 'monthLong'),
    sub: t('cards.sub.month', { year: start.getFullYear() }),
    hero: fmtCompact(r.totals.total_tokens),
    unit: t('cards.unit'),
    line: [runs(t('cards.apiEq', { v: fmtMoney(r.totals.cost_usd) })), ...multipleRun(multiple)],
    chart: { kind: 'bars', values, avg, ticks: [1, 8, 15, 22, 29].filter((d) => d <= values.length).map((d) => ({ i: d - 1, label: String(d) })) },
    left: peak ? runs(t('cards.peakDay', { d: dayMonth(new Date(peak.date + 'T00:00:00')) })) : [],
    right: avg !== null ? runs(t('cards.dailyAvg', { v: fmtCompact(avg) })) : [],
    avgLine: avg !== null,
    stats: [active, sessions, cache, topModel(r.by_model, r.totals.total_tokens)].filter((x): x is Stat => !!x),
    square: [active, sessions, cache],
    share: days > 0 ? r.active_days / days : 0,
  }
}

/** `prior` is the 30 days before, for how this day compares with an average day of use. */
export function buildDay(d: DayDetail, prior: Report, day: Date, now = new Date()): Card | null {
  if (d.totals.events === 0) return null
  const hours = Array.from({ length: 24 }, (_, h) => d.hourly.find((p) => p.hour === h)?.tokens ?? 0)
  const activeHours = hours.filter((v) => v > 0).length
  const peakHour = hours.indexOf(Math.max(...hours))
  const today = day.toDateString() === now.toDateString()
  const begun = today ? now.getHours() + 1 : 24
  const avg = prior.active_days > 0 ? prior.totals.total_tokens / prior.active_days : 0
  const sessions: Stat = {
    label: t('cards.stat.sessions'),
    value: fmtInt(d.sessions),
    detail: d.first_ms && d.last_ms ? t('cards.detail.span', { from: fmtClock(d.first_ms), to: fmtClock(d.last_ms) }) : '',
  }
  const requests: Stat = {
    label: t('cards.stat.requests'),
    value: fmtInt(d.totals.events),
    detail: t('cards.detail.perHour', { v: fmtInt(Math.round(d.totals.events / Math.max(1, activeHours))) }),
  }
  const cache = cacheStat(d.totals.tokens)
  const date = new Intl.DateTimeFormat(locale(), { day: 'numeric', month: 'long', year: 'numeric' }).format(day)
  return {
    eyebrow: upper(t('cards.eyebrow.day')),
    title: new Intl.DateTimeFormat(locale(), { weekday: 'long' }).format(day),
    squareTitle: date,
    sub: date,
    hero: fmtCompact(d.totals.total_tokens),
    unit: t('cards.unit'),
    line: [runs(t('cards.apiEq', { v: fmtMoney(d.totals.cost_usd) })), ...(avg > 0 ? [runs(t('cards.vsAverage', { n: fmtDec(d.totals.total_tokens / avg, 1) }))] : [])],
    chart: { kind: 'bars', values: hours, avg: null, ticks: [0, 6, 12, 18, 23].map((h) => ({ i: h, label: String(h).padStart(2, '0') })) },
    left: runs(t('cards.peakHour', { h: fmtHour(peakHour) })),
    right: runs(t('cards.activeHours', { n: activeHours })),
    avgLine: false,
    stats: [sessions, requests, cache, topModel(d.by_model, d.totals.total_tokens)].filter((x): x is Stat => !!x),
    square: [sessions, requests, cache],
    share: Math.min(1, activeHours / begun),
  }
}
