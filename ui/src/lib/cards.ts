import type { CardInfo, DayDetail, DayPoint, Earlier, Group, Report, Tokens } from './api'
import { fmtClock, fmtCompact, fmtDate, fmtDec, fmtHour, fmtInt, fmtMoney, fmtPct, i18n, t, toolLabel } from './i18n.svelte'

export type CardKind = 'day' | 'week' | 'month' | 'year'
export type Design = 'summary' | 'calendar' | 'hours' | 'wrapped'

/** A piece of a line; `**bold**` in a string marks the bold pieces. */
export interface Run {
  text: string
  bold: boolean
  accent?: boolean
}
export interface Stat {
  label: string
  value: string
  detail: string
}
export interface Share {
  label: string
  share: number
  pct: string
}
export type Glyph = 'streak' | 'everyday' | 'tokens' | 'cache' | 'plan' | 'tools'
export interface Badge {
  glyph: Glyph
  tier: 1 | 2 | 3
  title: string
  detail: string
}

export type Chart =
  | { kind: 'strip'; levels: number[]; offset: number; ticks: { col: number; label: string }[] }
  | { kind: 'bars'; values: number[]; avg: number | null; ticks: { i: number; label: string }[] }
  /** One month as a calendar; a level of -1 is a day still to come. */
  | { kind: 'month'; levels: number[]; offset: number; weekdays: string[]; today: number | null }
  | { kind: 'year'; months: { name: string; levels: number[]; offset: number }[] }
  /** Tokens by hour of day around a dial; `window` is the six hours the chronotype names. */
  | { kind: 'clock'; values: number[]; window: number | null; center: string; caption: string }
  | { kind: 'punch'; values: number[][]; weekdays: string[] }
  | { kind: 'ranking'; rows: { label: string; value: string; share: number }[] }
  | { kind: 'badges'; badges: Badge[] }

export interface Note {
  left: Run[]
  right: Run[]
  legend: 'heat' | 'peak'
  avgLine: boolean
  less: string
  more: string
}

/** Everything a card shows, in the app's language. */
export interface Card {
  eyebrow: string
  /** "2 / 5" on a year-in-review slide. */
  pager: string
  title: string
  squareTitle: string
  sub: string
  hero: { value: string; unit: string } | null
  /** Lines of items; the items of a line sit side by side. */
  lines: Run[][][]
  chart: Chart | null
  /** A second chart under the first where there is room: the week-by-hour grid of the Hours card. */
  extra: Chart | null
  note: Note | null
  stats: Stat[]
  square: Stat[]
  tools: Share[]
  badges: Badge[]
  /** How much of the dial is filled: active days or hours out of those that have begun; `null` leaves it off. */
  share: number | null
}

/** What a card is built from; the page loads it for the chosen period. */
export interface Facts {
  kind: CardKind
  from: Date
  /** The period's last day. */
  to: Date
  now: Date
  report: Report
  /** The day card's hours. */
  day: DayDetail | null
  /** The 30 days before a day card's day, for how it compares with an average day of use. */
  prior: Report | null
  sessions: number | null
  /** Days of the period that have begun since the archive's first request. */
  begun: number
  multiple: number | null
  earlier: Earlier | null
  hide: CardInfo[]
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
  const sums = monthSums(days)
  let best: { month: number; tokens: number } | null = null
  sums.forEach((tokens, month) => {
    if (tokens > 0 && (!best || tokens > best.tokens)) best = { month, tokens }
  })
  return best
}

function monthSums(days: DayPoint[]): number[] {
  const sums = Array<number>(12).fill(0)
  for (const d of days) sums[Number(d.date.slice(5, 7)) - 1] += d.tokens
  return sums
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

const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n)

/** The day, week (Monday first), month or year around `d`, as its first and last day. */
export function periodBounds(kind: CardKind, d: Date): { from: Date; to: Date } {
  const day = new Date(d.getFullYear(), d.getMonth(), d.getDate())
  if (kind === 'day') return { from: day, to: day }
  if (kind === 'week') {
    const from = addDays(day, -((day.getDay() + 6) % 7))
    return { from, to: addDays(from, 6) }
  }
  if (kind === 'month') return { from: new Date(d.getFullYear(), d.getMonth(), 1), to: new Date(d.getFullYear(), d.getMonth() + 1, 0) }
  return { from: new Date(d.getFullYear(), 0, 1), to: new Date(d.getFullYear(), 11, 31) }
}

/** The period of the same kind right before the one starting at `from`. */
export function previousPeriod(kind: CardKind, from: Date): { from: Date; to: Date } {
  if (kind === 'day') return periodBounds('day', addDays(from, -1))
  if (kind === 'week') return periodBounds('week', addDays(from, -7))
  if (kind === 'month') return periodBounds('month', new Date(from.getFullYear(), from.getMonth() - 1, 1))
  return periodBounds('year', new Date(from.getFullYear() - 1, 0, 1))
}

export function designsFor(kind: CardKind): Design[] {
  return kind === 'year' ? ['summary', 'calendar', 'hours', 'wrapped'] : kind === 'month' ? ['summary', 'calendar', 'hours'] : ['summary', 'hours']
}

export type Chronotype = 'morning' | 'day' | 'evening' | 'night' | 'balanced'
/** The six-hour stretches a chronotype names, by their first hour. */
const WINDOWS: [Exclude<Chronotype, 'balanced'>, number][] = [
  ['night', 23],
  ['morning', 5],
  ['day', 11],
  ['evening', 17],
]

/** Which six hours hold most of the use. A stretch names it only with at least 40%, well above the 25% of an even spread. */
export function chronotype(hours: number[]): { type: Chronotype; start: number | null; share: number } {
  const total = hours.reduce((a, v) => a + v, 0)
  if (total <= 0) return { type: 'balanced', start: null, share: 0 }
  const sums = WINDOWS.map(([type, start]) => ({ type, start, share: [0, 1, 2, 3, 4, 5].reduce((a, i) => a + hours[(start + i) % 24], 0) / total }))
  const best = sums.reduce((a, b) => (b.share > a.share ? b : a))
  return best.share >= 0.4 ? best : { type: 'balanced', start: null, share: best.share }
}

const TOKEN_TIERS = [1e8, 1e9, 1e10]
const STREAK_TIERS = [7, 30, 100]
const CACHE_TIERS = [80, 90, 95]
const PLAN_TIERS = [2, 5, 10]
const TOOL_TIERS = [2, 3, 4]
const tierOf = (v: number, tiers: number[]) => tiers.filter((x) => v >= x).length as 0 | 1 | 2 | 3

/** Badges the period earned, the hardest to reach first. Each stands for a fixed threshold the data reached. */
export function badgesFor(f: Facts): Badge[] {
  const shows = (x: CardInfo) => !f.hide.includes(x)
  const r = f.report
  const out: Badge[] = []
  const add = (glyph: Glyph, tier: number, title: string, detail: string) => tier > 0 && out.push({ glyph, tier: tier as 1 | 2 | 3, title, detail })

  const tokens = tierOf(r.totals.total_tokens, TOKEN_TIERS)
  add('tokens', tokens, t(`badge.tokens.${tokens}`), t('badge.tokens.detail', { v: fmtCompact(r.totals.total_tokens) }))
  if (f.kind === 'month' || f.kind === 'year') {
    const streak = longestStreak(r.daily)
    const tier = tierOf(streak, STREAK_TIERS)
    add('streak', tier, t('badge.streak', { n: STREAK_TIERS[tier - 1] ?? 0 }), t('badge.streak.detail', { n: streak }))
  }
  const ended = addDays(f.to, 1) <= f.now
  if ((f.kind === 'week' || f.kind === 'month') && ended && f.begun === r.days_in_range && r.active_days === r.days_in_range) {
    add('everyday', f.kind === 'week' ? 1 : 2, t(`badge.everyday.${f.kind}`), t('badge.everyday.detail', { n: r.days_in_range }))
  }
  if (shows('plan') && f.multiple !== null) {
    const tier = tierOf(f.multiple, PLAN_TIERS)
    add('plan', tier, t('badge.plan', { n: PLAN_TIERS[tier - 1] ?? 0 }), t('badge.plan.detail', { n: fmtDec(f.multiple, 1) }))
  }
  const k = r.totals.tokens
  const hit = cacheHit(k)
  if (hit !== null && k.input + k.cache_read + k.cache_write >= 1e6) {
    const tier = tierOf(hit, CACHE_TIERS)
    add('cache', tier, t('badge.cache', { pct: fmtPct(CACHE_TIERS[tier - 1] ?? 0) }), t('badge.cache.detail', { pct: fmtPct(hit) }))
  }
  if (shows('tools')) {
    const used = r.by_tool.filter((g) => g.totals.total_tokens > 0)
    const tier = tierOf(used.length, TOOL_TIERS)
    add('tools', tier, t('badge.tools', { n: used.length }), used.map((g) => toolLabel(g.key)).join(', '))
  }
  // a tier counts for more on the badges that take more use to reach
  const weight: Record<Glyph, number> = { tokens: 1, streak: 1, everyday: 0.5, plan: 0.5, cache: 0, tools: -0.5 }
  const order = Object.keys(weight)
  const score = (b: Badge) => b.tier + weight[b.glyph]
  return out.sort((a, b) => score(b) - score(a) || order.indexOf(a.glyph) - order.indexOf(b.glyph))
}

/** Each tool's share of the tokens, largest first. */
export function toolShares(groups: Group[], total: number): Share[] {
  if (total <= 0) return []
  return groups
    .filter((g) => g.totals.total_tokens > 0)
    .sort((a, b) => b.totals.total_tokens - a.totals.total_tokens)
    .map((g) => ({ label: toolLabel(g.key), share: g.totals.total_tokens / total, pct: fmtPct((g.totals.total_tokens / total) * 100) }))
}

/** The tokens of this period against the same stretch of the one before, when the archive holds all of that one. */
export function changeLine(f: Facts): Run[] | null {
  const e = f.earlier
  if (f.hide.includes('change') || !e?.covered || e.totals.total_tokens <= 0) return null
  const pct = (f.report.totals.total_tokens / e.totals.total_tokens - 1) * 100
  const running = f.from <= f.now && f.now < addDays(f.to, 1)
  const basis = t(`cards.change.${f.kind}${running ? '.running' : ''}`)
  if (Math.abs(pct) < 1) return runs(t('cards.change.same', { basis }))
  const arrow = pct > 0 ? '↑' : '↓'
  return [{ text: `${arrow} ${fmtPct(Math.abs(pct), Math.abs(pct) < 10 ? 1 : 0)}`, bold: true, accent: true }, ...runs(` ${basis}`)]
}

const locale = () => (i18n.lang === 'tr' ? 'tr-TR' : 'en-US')
const upper = (s: string) => s.toLocaleUpperCase(locale())
const monthName = (m: number, year = 2000) => new Intl.DateTimeFormat(locale(), { month: 'long' }).format(new Date(year, m, 1))
const dayMonth = (d: Date) => new Intl.DateTimeFormat(locale(), { day: 'numeric', month: 'long' }).format(d)
const weekdayName = (i: number, style: 'long' | 'short' | 'narrow' = 'long') =>
  new Intl.DateTimeFormat(locale(), { weekday: style }).format(new Date(2024, 0, 1 + i))
const weekdays = (style: 'short') => [0, 1, 2, 3, 4, 5, 6].map((i) => weekdayName(i, style))
const date = (iso: string) => new Date(iso + 'T00:00:00')
/** Monday-first weekday of `d`, 0–6. */
const mondayIndex = (d: Date) => (d.getDay() + 6) % 7

export function weekTitle(from: Date, to: Date, year = false): string {
  return new Intl.DateTimeFormat(locale(), { day: 'numeric', month: 'long', ...(year ? { year: 'numeric' } : {}) }).formatRange(from, to)
}

function topModel(groups: Group[], total: number): Stat | null {
  const g = [...groups].sort((a, b) => b.totals.total_tokens - a.totals.total_tokens)[0]
  if (!g || total <= 0) return null
  return { label: t('cards.stat.model'), value: g.label, detail: t('cards.detail.modelShare', { pct: fmtPct((g.totals.total_tokens / total) * 100) }) }
}

function requestsStat(r: Report, days: number): Stat {
  return { label: t('cards.stat.requests'), value: fmtInt(r.totals.events), detail: t('cards.detail.perDay', { v: fmtInt(Math.round(r.totals.events / Math.max(1, days))) }) }
}

const cacheStat = (k: Tokens): Stat => {
  const hit = cacheHit(k)
  return { label: t('cards.stat.cache'), value: hit !== null ? fmtPct(hit) : '—', detail: t('cards.detail.cache') }
}

function sharedParts(f: Facts) {
  const shows = (x: CardInfo) => !f.hide.includes(x)
  const r = f.report
  const money: Run[][] = [
    ...(shows('cost') ? [runs(t('cards.apiEq', { v: fmtMoney(r.totals.cost_usd) }))] : []),
    ...(shows('plan') && f.multiple !== null ? [runs(t('cards.multiple', { n: fmtDec(f.multiple, f.multiple < 10 ? 1 : 0) }))] : []),
  ]
  const change = changeLine(f)
  return {
    shows,
    money,
    change: change ? [[change]] : [],
    model: shows('models') ? topModel(r.by_model, r.totals.total_tokens) : null,
    tools: shows('tools') ? toolShares(r.by_tool, r.totals.total_tokens) : [],
    badges: shows('badges') ? badgesFor(f) : [],
  }
}

const heatNote = (right: Run[]): Note => ({ left: [], right, legend: 'heat', avgLine: false, less: t('cards.less'), more: t('cards.more') })
const peakNote = (left: Run[], right: Run[], avgLine: boolean): Note => ({ left, right, legend: 'peak', avgLine, less: '', more: '' })

function hoursOf(f: Facts): number[] {
  if (f.day) return Array.from({ length: 24 }, (_, h) => f.day!.hourly.find((p) => p.hour === h)?.tokens ?? 0)
  return Array.from({ length: 24 }, (_, h) => f.report.heatmap.reduce((a, row) => a + (row[h] ?? 0), 0))
}

export function buildSummary(f: Facts): Card | null {
  const r = f.report
  if (r.totals.events === 0) return null
  const p = sharedParts(f)
  const hero = { value: fmtCompact(r.totals.total_tokens), unit: t('cards.unit') }
  const base = { pager: '', hero, extra: null, tools: p.tools, badges: p.badges }

  if (f.kind === 'day') {
    const d = f.day
    if (!d) return null
    const hours = hoursOf(f)
    const active = hours.filter((v) => v > 0).length
    const today = f.from.toDateString() === f.now.toDateString()
    const begun = today ? f.now.getHours() + 1 : 24
    const avg = f.prior && f.prior.active_days > 0 ? f.prior.totals.total_tokens / f.prior.active_days : 0
    const sessions: Stat = {
      label: t('cards.stat.sessions'),
      value: fmtInt(d.sessions),
      detail: d.first_ms && d.last_ms ? t('cards.detail.span', { from: fmtClock(d.first_ms), to: fmtClock(d.last_ms) }) : '',
    }
    const requests: Stat = { label: t('cards.stat.requests'), value: fmtInt(d.totals.events), detail: t('cards.detail.perHour', { v: fmtInt(Math.round(d.totals.events / Math.max(1, active))) }) }
    const cache = cacheStat(d.totals.tokens)
    const full = new Intl.DateTimeFormat(locale(), { day: 'numeric', month: 'long', year: 'numeric' }).format(f.from)
    const vsAvg = avg > 0 ? [runs(t('cards.vsAverage', { n: fmtDec(d.totals.total_tokens / avg, 1) }))] : []
    return {
      ...base,
      eyebrow: upper(t('cards.eyebrow.day')),
      title: weekdayName(mondayIndex(f.from)),
      squareTitle: full,
      sub: full,
      lines: [[...p.money, ...vsAvg], ...p.change].filter((l) => l.length),
      chart: { kind: 'bars', values: hours, avg: null, ticks: [0, 6, 12, 18, 23].map((h) => ({ i: h, label: String(h).padStart(2, '0') })) },
      note: peakNote(runs(t('cards.peakHour', { h: fmtHour(hours.indexOf(Math.max(...hours))) })), runs(t('cards.activeHours', { n: active })), false),
      stats: [sessions, requests, cache, p.model ?? requests].filter((s, i, a) => a.indexOf(s) === i),
      square: [sessions, requests, cache],
      share: Math.min(1, active / begun),
    }
  }

  const values = r.daily.map((d) => d.tokens)
  const used = r.daily.filter((d) => d.events > 0)
  const avg = used.length ? used.reduce((a, d) => a + d.tokens, 0) / used.length : null
  const peak = r.peak_day
  const streak = longestStreak(r.daily)
  const share = f.begun > 0 ? r.active_days / f.begun : 0
  const active: Stat = {
    label: t('cards.stat.active'),
    value: `${fmtInt(r.active_days)} / ${fmtInt(f.kind === 'week' ? 7 : f.begun)}`,
    detail: f.kind === 'year' ? t('cards.detail.daysShare', { pct: fmtPct(share * 100) }) : t('cards.detail.streak', { n: streak }),
  }
  const sessions: Stat | null =
    f.sessions !== null ? { label: t('cards.stat.sessions'), value: fmtInt(f.sessions), detail: t('cards.detail.perDay', { v: fmtDec(f.sessions / Math.max(1, r.active_days), 1) }) } : null
  const cache = cacheStat(r.totals.tokens)
  const requests = requestsStat(r, r.active_days)
  const lines = [p.money, ...p.change].filter((l) => l.length)

  if (f.kind === 'week' || f.kind === 'month') {
    const week = f.kind === 'week'
    const peakText = peak ? (week ? weekdayName(mondayIndex(date(peak.date))) : dayMonth(date(peak.date))) : ''
    const second = sessions ?? requests
    return {
      ...base,
      eyebrow: upper(t(`cards.eyebrow.${f.kind}`)),
      title: week ? weekTitle(f.from, f.to) : monthName(f.from.getMonth(), f.from.getFullYear()),
      squareTitle: week ? weekTitle(f.from, f.to) : fmtDate(f.from, 'monthLong'),
      sub: t(`cards.sub.${f.kind}`, { year: f.from.getFullYear() }),
      lines,
      chart: {
        kind: 'bars',
        values,
        avg,
        ticks: week ? weekdays('short').map((label, i) => ({ i, label })) : [1, 8, 15, 22, 29].filter((d) => d <= values.length).map((d) => ({ i: d - 1, label: String(d) })),
      },
      note: peakNote(peak ? runs(t('cards.peakDay', { d: peakText })) : [], avg !== null ? runs(t('cards.dailyAvg', { v: fmtCompact(avg) })) : [], avg !== null),
      stats: [active, second, cache, p.model ?? (second === requests ? null : requests)].filter((x): x is Stat => !!x),
      square: [active, second, cache],
      share,
    }
  }

  const year = f.from.getFullYear()
  const offset = mondayIndex(f.from)
  const ticks = [0, 3, 6, 9].map((m) => {
    const doy = Math.round((new Date(year, m, 1).getTime() - f.from.getTime()) / 86_400_000)
    return { col: Math.floor((doy + offset) / 7), label: new Intl.DateTimeFormat(locale(), { month: 'short' }).format(new Date(year, m, 1)) }
  })
  const pm = peakMonth(r.daily)
  const month: Stat | null = pm ? { label: t('cards.stat.peakMonth'), value: monthName(pm.month, year), detail: t('cards.detail.tokens', { v: fmtCompact(pm.tokens) }) } : null
  const streakStat: Stat = { label: t('cards.stat.streak'), value: t('days.streakValue', { n: streak }), detail: t('cards.detail.inARow') }
  const peakStat: Stat | null = peak
    ? { label: t('cards.stat.peakDay'), value: dayMonth(date(peak.date)), detail: p.shows('cost') ? t('cards.detail.tokensCost', { v: fmtCompact(peak.tokens), usd: fmtMoney(peak.cost_usd) }) : t('cards.detail.tokens', { v: fmtCompact(peak.tokens) }) }
    : null
  return {
    ...base,
    eyebrow: upper(t('cards.eyebrow.year')),
    title: String(year),
    squareTitle: String(year),
    sub: t('cards.sub.year'),
    lines,
    chart: { kind: 'strip', levels: heatLevels(values), offset, ticks },
    note: heatNote(runs(t('cards.activeDays', { n: fmtInt(r.active_days) }))),
    stats: [month, peakStat, streakStat, p.model ?? requests].filter((s): s is Stat => !!s),
    square: [active, month, streakStat].filter((s): s is Stat => !!s),
    share,
  }
}

/** Levels for each day of the period, with days still to come at -1. */
function dayLevels(f: Facts): number[] {
  const levels = heatLevels(f.report.daily.map((d) => d.tokens))
  return f.report.daily.map((d, i) => (date(d.date) > f.now ? -1 : levels[i]))
}

export function buildCalendar(f: Facts): Card | null {
  const r = f.report
  if (r.totals.events === 0 || (f.kind !== 'month' && f.kind !== 'year')) return null
  const p = sharedParts(f)
  const levels = dayLevels(f)
  const streak = longestStreak(r.daily)
  const share = f.begun > 0 ? r.active_days / f.begun : 0
  const tokens: Stat = { label: t('cards.stat.tokens'), value: fmtCompact(r.totals.total_tokens), detail: p.shows('cost') ? t('cards.detail.apiEq', { v: fmtMoney(r.totals.cost_usd) }) : '' }
  const streakStat: Stat = { label: t('cards.stat.streak'), value: t('days.streakValue', { n: streak }), detail: t('cards.detail.inARow') }
  const peak = r.peak_day
  const peakStat: Stat | null = peak ? { label: t('cards.stat.peakDay'), value: dayMonth(date(peak.date)), detail: t('cards.detail.tokens', { v: fmtCompact(peak.tokens) }) } : null
  const base = {
    eyebrow: upper(t('cards.eyebrow.calendar')),
    pager: '',
    hero: { value: fmtInt(r.active_days), unit: t('cards.unit.activeDays', { n: r.active_days }) },
    lines: [[runs(t('cards.daysShare', { pct: fmtPct(share * 100) }))]],
    extra: null,
    note: heatNote(runs(t('cards.longestStreak', { n: streak }))),
    tools: [],
    badges: p.badges.filter((b) => b.glyph === 'streak' || b.glyph === 'everyday'),
    share,
  }
  if (f.kind === 'month') {
    const today = f.now.getFullYear() === f.from.getFullYear() && f.now.getMonth() === f.from.getMonth() ? f.now.getDate() - 1 : null
    return {
      ...base,
      title: monthName(f.from.getMonth(), f.from.getFullYear()),
      squareTitle: fmtDate(f.from, 'monthLong'),
      sub: t('cards.sub.calendar', { year: f.from.getFullYear() }),
      chart: { kind: 'month', levels, offset: mondayIndex(f.from), weekdays: weekdays('short'), today },
      stats: [tokens, peakStat, streakStat].filter((s): s is Stat => !!s),
      square: [tokens, peakStat, streakStat].filter((s): s is Stat => !!s),
    }
  }
  const year = f.from.getFullYear()
  const pm = peakMonth(r.daily)
  const month: Stat | null = pm ? { label: t('cards.stat.peakMonth'), value: monthName(pm.month, year), detail: t('cards.detail.tokens', { v: fmtCompact(pm.tokens) }) } : null
  return {
    ...base,
    title: String(year),
    squareTitle: String(year),
    sub: t('cards.sub.calendarYear'),
    chart: yearChart(f, levels),
    stats: [tokens, month, peakStat, streakStat].filter((s): s is Stat => !!s),
    square: [tokens, month, streakStat].filter((s): s is Stat => !!s),
  }
}

function yearChart(f: Facts, levels: number[]): Chart {
  const year = f.from.getFullYear()
  const months = Array.from({ length: 12 }, (_, m) => {
    const first = new Date(year, m, 1)
    const start = Math.round((first.getTime() - f.from.getTime()) / 86_400_000)
    const days = new Date(year, m + 1, 0).getDate()
    return { name: new Intl.DateTimeFormat(locale(), { month: 'short' }).format(first), levels: levels.slice(start, start + days), offset: mondayIndex(first) }
  })
  return { kind: 'year', months }
}

function periodText(f: Facts): string {
  if (f.kind === 'day') return dayMonth(f.from)
  if (f.kind === 'week') return weekTitle(f.from, f.to)
  if (f.kind === 'month') return fmtDate(f.from, 'monthLong')
  return String(f.from.getFullYear())
}

export function buildHours(f: Facts): Card | null {
  const r = f.report
  if (r.totals.events === 0) return null
  const hours = hoursOf(f)
  const total = hours.reduce((a, v) => a + v, 0)
  if (total <= 0) return null
  const peak = hours.indexOf(Math.max(...hours))
  const c = chronotype(hours)
  const title = t(`cards.chrono.${c.type}`)
  const sub = c.start !== null ? t('cards.chrono.detail', { pct: fmtPct(c.share * 100), from: fmtHour(c.start), to: fmtHour((c.start + 6) % 24) }) : t('cards.chrono.balanced.detail', { h: fmtHour(peak) })
  const active = hours.filter((v) => v > 0).length
  const peakStat: Stat = { label: t('cards.stat.peakHour'), value: fmtHour(peak), detail: t('cards.detail.modelShare', { pct: fmtPct((hours[peak] / total) * 100) }) }
  const activeStat: Stat = { label: t('cards.stat.activeHours'), value: `${active} / 24`, detail: t('cards.detail.activeHours') }
  let second: Stat
  let extra: Chart | null = null
  if (f.day) {
    second = { label: t('cards.stat.span'), value: f.day.first_ms && f.day.last_ms ? `${fmtClock(f.day.first_ms)}–${fmtClock(f.day.last_ms)}` : '—', detail: t('cards.detail.firstLast') }
  } else {
    const byDay = r.heatmap.map((row) => row.reduce((a, v) => a + v, 0))
    const wd = byDay.indexOf(Math.max(...byDay))
    second = { label: t('cards.stat.peakWeekday'), value: weekdayName(wd), detail: t('cards.detail.modelShare', { pct: fmtPct((byDay[wd] / total) * 100) }) }
    extra = { kind: 'punch', values: r.heatmap, weekdays: weekdays('short') }
  }
  return {
    eyebrow: upper(`${periodText(f)} · ${t('cards.eyebrow.hours')}`),
    pager: '',
    title,
    squareTitle: title,
    sub,
    hero: null,
    lines: [],
    chart: { kind: 'clock', values: hours, window: c.start, center: fmtHour(peak), caption: t('cards.clock.caption') },
    extra,
    note: null,
    stats: [peakStat, second, activeStat],
    square: [peakStat, second, activeStat],
    tools: [],
    badges: [],
    share: null,
  }
}

/** The year in a row of story slides. Slides whose subject is hidden or empty are left out. */
export function buildWrapped(f: Facts): Card[] {
  const r = f.report
  if (f.kind !== 'year' || r.totals.events === 0) return []
  const p = sharedParts(f)
  const year = f.from.getFullYear()
  const share = f.begun > 0 ? r.active_days / f.begun : 0
  const slides: Card[] = []
  const blank = { pager: '', squareTitle: '', hero: null, lines: [], extra: null, note: null, stats: [], square: [], tools: [], badges: [], share: null }

  const months = monthSums(r.daily)
  const pm = peakMonth(r.daily)
  const used = months.filter((v) => v > 0)
  slides.push({
    ...blank,
    eyebrow: upper(t('cards.eyebrow.wrapped')),
    title: String(year),
    sub: t('cards.sub.year'),
    hero: { value: fmtCompact(r.totals.total_tokens), unit: t('cards.unit') },
    lines: [p.money, ...p.change].filter((l) => l.length),
    chart: { kind: 'bars', values: months, avg: used.length ? used.reduce((a, v) => a + v, 0) / used.length : null, ticks: months.map((_, m) => ({ i: m, label: monthName(m, year).slice(0, 1).toLocaleUpperCase(locale()) })) },
    note: peakNote(pm ? runs(t('cards.peakMonthLine', { m: monthName(pm.month, year) })) : [], runs(t('cards.activeDays', { n: fmtInt(r.active_days) })), false),
    stats: [
      { label: t('cards.stat.active'), value: `${fmtInt(r.active_days)} / ${fmtInt(f.begun)}`, detail: t('cards.detail.daysShare', { pct: fmtPct(share * 100) }) },
      requestsStat(r, r.active_days),
      cacheStat(r.totals.tokens),
      ...(p.tools[0] ? [{ label: t('cards.stat.topTool'), value: p.tools[0].label, detail: t('cards.detail.modelShare', { pct: fmtPct(p.tools[0].share * 100) }) }] : []),
    ],
    tools: p.tools,
    share,
  })

  const calendar = buildCalendar(f)
  if (calendar) {
    const streak = longestStreak(r.daily)
    slides.push({
      ...calendar,
      eyebrow: upper(t('cards.eyebrow.calendar')),
      title: t('cards.wrapped.days', { n: fmtInt(r.active_days) }),
      sub: t('cards.wrapped.daysSub', { pct: fmtPct(share * 100), n: streak }),
      hero: null,
      lines: [],
      stats: calendar.stats.filter((s) => s.label !== t('cards.stat.tokens')),
      badges: [],
    })
  }

  const hours = buildHours(f)
  if (hours) slides.push({ ...hours, eyebrow: upper(t('cards.eyebrow.rhythm')) })

  if (p.shows('models')) {
    const models = [...r.by_model].filter((g) => g.totals.total_tokens > 0).sort((a, b) => b.totals.total_tokens - a.totals.total_tokens)
    const top = models[0]
    if (top) {
      const total = r.totals.total_tokens
      slides.push({
        ...blank,
        eyebrow: upper(t('cards.eyebrow.models')),
        title: top.label,
        sub: t('cards.wrapped.modelSub', { pct: fmtPct((top.totals.total_tokens / total) * 100), n: models.length }),
        chart: { kind: 'ranking', rows: models.slice(0, 5).map((g) => ({ label: g.label, value: fmtPct((g.totals.total_tokens / total) * 100, 1), share: g.totals.total_tokens / total })) },
        tools: p.tools,
      })
    }
  }

  if (p.badges.length) {
    slides.push({
      ...blank,
      eyebrow: upper(t('cards.eyebrow.badges')),
      title: t('cards.wrapped.badges', { n: p.badges.length }),
      sub: t('cards.wrapped.badgesSub'),
      chart: { kind: 'badges', badges: p.badges.slice(0, 6) },
    })
  }

  return slides.map((s, i) => ({ ...s, squareTitle: s.title, pager: `${i + 1} / ${slides.length}` }))
}
