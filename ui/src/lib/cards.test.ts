import { beforeEach, describe, expect, it } from 'vitest'
import { badgesFor, changeLine, chronotype, heatLevels, longestStreak, peakMonth, periodBounds, previousPeriod, runs, type Facts } from './cards'
import type { DayPoint, Group, Report, Totals } from './api'
import { i18n } from './i18n.svelte'

const day = (date: string, tokens: number): DayPoint => ({
  date, tokens, cost_usd: 0, events: tokens > 0 ? 1 : 0, by_tool: {}, cost_by_tool: {}, prompt_tokens: 0, cache_read: 0, cache_write: 0, cache_savings_usd: 0,
})

describe('summary cards', () => {
  it('splits bold pieces out of a line', () => {
    expect(runs('**$8,412** API equivalent')).toEqual([{ text: '$8,412', bold: true }, { text: ' API equivalent', bold: false }])
    expect(runs('Worth **4.6×** the plan price')).toEqual([{ text: 'Worth ', bold: false }, { text: '4.6×', bold: true }, { text: ' the plan price', bold: false }])
  })

  it('counts the longest run of days with use', () => {
    expect(longestStreak([1, 1, 0, 1, 1, 1, 0].map((events) => ({ events })))).toBe(3)
    expect(longestStreak([])).toBe(0)
  })

  it('finds the month with the most tokens', () => {
    expect(peakMonth([day('2026-01-05', 10), day('2026-02-01', 4), day('2026-02-02', 9), day('2026-03-01', 0)])).toEqual({ month: 1, tokens: 13 })
    expect(peakMonth([day('2026-01-05', 0)])).toBeNull()
  })

  it('grades days with use by quartile and leaves idle days at zero', () => {
    expect(heatLevels([0, 1, 2, 3, 4, 0])).toEqual([0, 1, 2, 3, 4, 0])
    expect(heatLevels([0, 5, 5, 5])).toEqual([0, 1, 1, 1])
    expect(heatLevels([0, 0])).toEqual([0, 0])
  })
})

const totals = (tokens: number, k: Partial<Totals['tokens']> = {}): Totals => ({
  events: tokens > 0 ? 10 : 0,
  tokens: { input: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, output: 0, reasoning: 0, ...k },
  total_tokens: tokens,
  cost: { input: 0, output: 0, cache_read: 0, cache_write: 0, web_search: 0 },
  cost_usd: 0,
  unpriced_events: 0,
  unpriced_tokens: 0,
  cache_read_with_writes: 0,
  cache_write_with_reads: 0,
  cache_savings_usd: 0,
})
const group = (key: string, tokens: number): Group => ({ key, label: key, hidden: false, totals: totals(tokens) })
const ymd = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`

/** A month of `daily` token counts starting on 2026-10-01. */
function facts(daily: number[], o: Partial<Facts> & { cache?: Partial<Totals['tokens']>; tools?: Group[] } = {}): Facts {
  const days = daily.map((tokens, i) => day(ymd(new Date(2026, 9, 1 + i)), tokens))
  const sum = daily.reduce((a, v) => a + v, 0)
  const report = {
    totals: totals(sum, o.cache),
    by_tool: o.tools ?? [group('claude_code', sum)],
    by_model: [],
    daily: days,
    heatmap: [],
    days_in_range: days.length,
    active_days: days.filter((d) => d.tokens > 0).length,
  } as unknown as Report
  return {
    kind: 'month', from: new Date(2026, 9, 1), to: new Date(2026, 9, daily.length), now: new Date(2026, 10, 3, 9), report, day: null, prior: null,
    sessions: null, begun: daily.length, multiple: null, earlier: null, hide: [], ...o,
  }
}

describe('periods', () => {
  it('starts a week on Monday', () => {
    const { from, to } = periodBounds('week', new Date(2026, 10, 4))
    expect([ymd(from), ymd(to)]).toEqual(['2026-11-02', '2026-11-08'])
    expect(ymd(periodBounds('week', new Date(2026, 10, 8)).from)).toBe('2026-11-02')
  })

  it('compares with the calendar period right before', () => {
    const month = previousPeriod('month', new Date(2026, 0, 1))
    expect([ymd(month.from), ymd(month.to)]).toEqual(['2025-12-01', '2025-12-31'])
    const feb = previousPeriod('month', new Date(2026, 2, 1))
    expect([ymd(feb.from), ymd(feb.to)]).toEqual(['2026-02-01', '2026-02-28'])
    expect(ymd(previousPeriod('week', new Date(2026, 10, 2)).from)).toBe('2026-10-26')
    expect(ymd(previousPeriod('year', new Date(2026, 0, 1)).to)).toBe('2025-12-31')
  })
})

describe('card facts', () => {
  beforeEach(() => {
    i18n.lang = 'en'
  })

  it('names the six hours that hold most of the use, and only when they stand out', () => {
    const night = Array(24).fill(1)
    for (const h of [23, 0, 1, 2]) night[h] = 10
    expect(chronotype(night)).toMatchObject({ type: 'night', start: 23 })
    expect(chronotype(Array(24).fill(1))).toMatchObject({ type: 'balanced', start: null })
    expect(chronotype(Array(24).fill(0))).toEqual({ type: 'balanced', start: null, share: 0 })
  })

  it('compares with the period before only when the archive holds all of it', () => {
    const f = facts([100, 100, 100])
    const earlier = (t: number, covered = true) => ({ totals: totals(t), covered })
    expect(changeLine({ ...f, earlier: earlier(200, false) })).toBeNull()
    expect(changeLine({ ...f, earlier: earlier(0) })).toBeNull()
    expect(changeLine({ ...f, earlier: earlier(200), hide: ['change'] })).toBeNull()
    const up = changeLine({ ...f, earlier: earlier(200) })!
    expect(up[0]).toEqual({ text: '↑ 50%', bold: true, accent: true })
    expect(up.map((r) => r.text).join('')).toBe('↑ 50% vs the month before')
    const running = changeLine({ ...f, now: new Date(2026, 9, 2, 9), earlier: earlier(400) })!
    expect(running.map((r) => r.text).join('')).toBe('↓ 25% vs the same part of the month before')
  })

  it('gives badges for fixed thresholds only, the highest first', () => {
    const month = Array(30).fill(4e7)
    month[3] = 0
    const titles = (f: Facts) => badgesFor(f).map((b) => b.title)
    // 1.16 billion tokens and a 26-day streak; the gap keeps "every day" away
    expect(titles(facts(month))).toEqual(['1 billion tokens', '7-day streak'])
    expect(titles(facts(Array(31).fill(4e7)))).toEqual(['1 billion tokens', '30-day streak', 'Every day of the month'])
    // a month still running cannot be used every day yet
    expect(titles(facts(Array(31).fill(4e7), { now: new Date(2026, 9, 31, 12) }))).not.toContain('Every day of the month')
    expect(titles(facts([5e7], { multiple: 6.2 }))).toContain('5× the plan')
    expect(titles(facts([5e7], { multiple: 6.2, hide: ['plan'] }))).not.toContain('5× the plan')
    expect(titles(facts([5e7], { cache: { input: 5e4, cache_read: 9.5e5 } }))).toContain('Cache 95%+')
    expect(titles(facts([5e7], { cache: { input: 5e3, cache_read: 9.5e4 } })), 'too little input to judge').not.toContain('Cache 95%+')
    expect(titles(facts([5e7], { tools: [group('claude_code', 3e7), group('codex', 2e7)] }))).toContain('2 tools together')
  })
})
