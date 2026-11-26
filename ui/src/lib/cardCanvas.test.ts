import { afterEach, describe, expect, it, vi } from 'vitest'
import { drawCard } from './cardCanvas'
import type { Card } from './cards'

afterEach(() => {
  vi.unstubAllGlobals()
})

/** A 2D context that keeps every rounded rectangle it is asked to draw. */
function recorder() {
  const rects: { x: number; w: number }[] = []
  let font = '400 10px x'
  const gradient = { addColorStop() {} }
  const ctx: Record<string, unknown> = {
    get font() {
      return font
    },
    set font(v: string) {
      font = v
    },
    measureText: (s: string) => ({ width: s.length * Number(/(\d+(?:\.\d+)?)px/.exec(font)?.[1] ?? 10) * 0.56 }),
    roundRect: (x: number, _y: number, w: number) => rects.push({ x, w }),
    createLinearGradient: () => gradient,
    createRadialGradient: () => gradient,
  }
  const proxy = new Proxy(ctx, { get: (t, k: string) => (k in t ? t[k] : () => {}) })
  return { rects, canvas: { width: 0, height: 0, getContext: () => proxy } as unknown as HTMLCanvasElement }
}

const card = (shares: number[]): Card => ({
  eyebrow: 'NOVEMBER 2026',
  pager: '',
  title: 'Your month',
  squareTitle: 'Your month',
  sub: '',
  hero: { value: '1,234,567', unit: 'tokens' },
  lines: [],
  chart: { kind: 'bars', values: Array.from({ length: 30 }, (_, i) => i), avg: 10, ticks: [] },
  extra: null,
  note: null,
  stats: [],
  square: [],
  tools: shares.map((share, i) => ({ label: `Tool ${i}`, share, pct: `${share * 100}%` })),
  badges: [],
  share: null,
})

describe('the share card', () => {
  it('keeps every tool share bar inside the content edge, however small a share', () => {
    vi.stubGlobal('Path2D', class {})
    const { rects, canvas } = recorder()
    drawCard(canvas, card([0.995, 0.0025, 0.0025]), { format: 'landscape', theme: 'navy', accent: null, family: 'x', screen: { w: 1920, h: 1080 } })
    expect(rects.length).toBeGreaterThan(0)
    for (const r of rects) {
      expect(r.w).toBeGreaterThanOrEqual(0)
      expect(r.x + r.w).toBeLessThanOrEqual(1140 + 1e-6)
    }
  })
})
