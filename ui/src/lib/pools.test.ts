import { describe, expect, it } from 'vitest'
import { poolRank } from './pools'

const fullest = (...xs: { state: string; used_pct: number | null; blocked: boolean }[]) => [...xs].sort((a, b) => poolRank(b) - poolRank(a))[0]

describe('poolRank', () => {
  it('keeps a pool with use since its reading above an untouched one', () => {
    const gemini = { state: 'behind', used_pct: 95, blocked: false }
    expect(fullest({ state: 'fresh', used_pct: 0, blocked: false }, gemini)).toBe(gemini)
  })

  it('counts a pool its week locked as full', () => {
    const locked = { state: 'fresh', used_pct: 10, blocked: true }
    expect(fullest(locked, { state: 'fresh', used_pct: 20, blocked: false })).toBe(locked)
  })

  it('puts a current reading above an old one', () => {
    const now = { state: 'fresh', used_pct: 5, blocked: false }
    expect(fullest({ state: 'stale', used_pct: 90, blocked: false }, now)).toBe(now)
  })
})
