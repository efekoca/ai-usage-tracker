import { describe, expect, it } from 'vitest'
import { axisPicks, labelWidth } from './axis'

// each picked label's left and right edge, placed as the chart sets them
function spans(picks: number[], n: number, iw: number, wide: number) {
  return picks.map((i) => {
    const x = n > 1 ? (i * iw) / (n - 1) : 0
    return i === 0 ? [x, x + wide] : i === n - 1 ? [x - wide, x] : [x - wide / 2, x + wide / 2]
  })
}

describe('axisPicks', () => {
  it('keeps twelve-hour times apart, the last one included', () => {
    const wide = labelWidth('04:00 PM')
    for (const iw of [300, 340, 420, 560, 700, 900]) {
      for (const n of [7, 13, 17, 24]) {
        const s = spans(axisPicks(n, iw, wide), n, iw, wide)
        for (let k = 1; k < s.length; k++) expect(s[k][0], `${n} points in ${iw}px`).toBeGreaterThanOrEqual(s[k - 1][1])
      }
    }
  })

  it('shows the last point when there is room for it', () => {
    const picks = axisPicks(24, 900, labelWidth('16:00'))
    expect(picks[0]).toBe(0)
    expect(picks.at(-1)).toBe(23)
  })
})
