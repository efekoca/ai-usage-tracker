import { describe, expect, it } from 'vitest'
import { heatLevels, longestStreak, peakMonth, runs } from './cards'
import type { DayPoint } from './api'

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
