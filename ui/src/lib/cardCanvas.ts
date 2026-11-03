import type { CardFormat, CardTheme } from './api'
import type { Badge, Card, Chart, Glyph, Note, Run, Share, Stat } from './cards'

export type Format = CardFormat
export type Theme = CardTheme

const FIXED = { story: { w: 1080, h: 1920 }, square: { w: 1080, h: 1080 }, landscape: { w: 1200, h: 675 } }

/** The wallpaper takes the screen's size in device pixels, within reason. */
export function sizeOf(format: Format, screen: { w: number; h: number }): { w: number; h: number } {
  if (format !== 'wallpaper') return FIXED[format]
  const scale = Math.min(1, 5120 / screen.w, 3200 / screen.h)
  const w = Math.round(Math.max(1280, screen.w * scale))
  return { w, h: Math.round(Math.max(720, (screen.h / screen.w) * w)) }
}

interface Palette {
  bg: [string, string]
  /** Soft light behind the content: center as a share of the width and height, radius as a share of the longer side. */
  glows: { x: number; y: number; r: number; color: string }[]
  ink: string
  ink2: string
  ink3: string
  track: string
  rule: string
  empty: string
  light: boolean
}

const THEMES: Record<Theme, Palette> = {
  navy: { bg: ['#0b1020', '#0b1020'], glows: [], ink: '#ffffff', ink2: '#b8c0d6', ink3: '#8a93ab', track: '#1a2244', rule: 'rgba(255,255,255,0.1)', empty: '#161d36', light: false },
  light: { bg: ['#f4f5f8', '#f4f5f8'], glows: [], ink: '#0e1220', ink2: '#3f4657', ink3: '#5f6576', track: '#dde2ee', rule: 'rgba(14,18,32,0.1)', empty: '#e3e7f0', light: true },
  sunset: {
    bg: ['#1a0d2b', '#4b1838'],
    glows: [
      { x: 1, y: 0, r: 0.75, color: 'rgba(255,122,69,0.42)' },
      { x: 0, y: 1, r: 0.65, color: 'rgba(255,72,140,0.24)' },
    ],
    ink: '#fff7f2', ink2: '#f2cfc6', ink3: '#c99ba0', track: 'rgba(255,255,255,0.12)', rule: 'rgba(255,255,255,0.14)', empty: 'rgba(255,255,255,0.09)', light: false,
  },
  aurora: {
    bg: ['#04121a', '#06202a'],
    glows: [
      { x: 0.05, y: 0, r: 0.8, color: 'rgba(38,214,170,0.32)' },
      { x: 1, y: 0.5, r: 0.7, color: 'rgba(124,92,255,0.28)' },
    ],
    ink: '#effffb', ink2: '#acd8d1', ink3: '#7ba69f', track: 'rgba(255,255,255,0.1)', rule: 'rgba(255,255,255,0.12)', empty: 'rgba(255,255,255,0.08)', light: false,
  },
}

/** Each theme's own accent; a chosen accent replaces it. */
export const THEME_ACCENT: Record<Theme, string> = { navy: '#3884ff', light: '#2f6feb', sunset: '#ff8a4c', aurora: '#3ee6b0' }
export const ACCENTS = ['#3884ff', '#8b7cff', '#3ee6b0', '#ffb547', '#ff8a4c', '#ff5c8a']
/** A swatch for the theme picker: its two background colors and its accent. */
export const themeSwatch = (theme: Theme) => ({ from: THEMES[theme].bg[0], to: THEMES[theme].glows[0]?.color.replace(/[\d.]+\)$/, '1)') ?? THEMES[theme].bg[1], accent: THEME_ACCENT[theme] })

function rgba(c: string): [number, number, number, number] {
  if (c.startsWith('#')) return [parseInt(c.slice(1, 3), 16), parseInt(c.slice(3, 5), 16), parseInt(c.slice(5, 7), 16), 1]
  const n = c.match(/[\d.]+/g)?.map(Number) ?? [0, 0, 0, 1]
  return [n[0], n[1], n[2], n[3] ?? 1]
}
function mix(a: string, b: string, t: number): string {
  const [x, y] = [rgba(a), rgba(b)]
  const v = x.map((p, i) => p + (y[i] - p) * t)
  return `rgba(${Math.round(v[0])},${Math.round(v[1])},${Math.round(v[2])},${v[3].toFixed(3)})`
}

function luminance(hex: string): number {
  const lin = (v: number) => (v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
  const [r, g, b] = rgba(hex)
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

interface Colors extends Palette {
  accent: string
  /** The accent where it is text: darker on the light theme so small type stays readable. */
  accentInk: string
  /** Text and marks drawn on the accent. */
  onAccent: string
  heat: string[]
  bar: string
  toolColors: string[]
}

function colors(theme: Theme, accent: string | null): Colors {
  const p = THEMES[theme]
  const a = accent ?? THEME_ACCENT[theme]
  return {
    ...p,
    accent: a,
    accentInk: p.light ? mix(a, p.ink, 0.3) : a,
    onAccent: luminance(a) > 0.45 ? '#0b1020' : '#ffffff',
    heat: [p.empty, mix(p.empty, a, 0.3), mix(p.empty, a, 0.55), mix(p.empty, a, 0.78), a],
    bar: mix(p.empty, a, 0.55),
    toolColors: [a, mix(a, p.ink, 0.45), mix(p.empty, a, 0.45), p.ink3, mix(p.ink3, p.empty, 0.5)],
  }
}

interface Spec {
  pad: number
  top: number
  bottom: number
  brand: number
  brandGap: number
  eyebrow: number
  title: number
  titleMax: number
  sub: number
  hero: number
  unit: number
  line: number
  label: number
  value: number
  small: number
  footer: number
  cols: number
  colGap: number
  rowGap: number
  medal: number
  bar: number
  /** Space between the main groups; spare room is added to it. */
  gap: number
  dial: { x: number; y: number; r: number; w: number } | null
}

const SPECS: Record<'story' | 'square' | 'wide', Spec> = {
  story: {
    pad: 96, top: 200, bottom: 200, brand: 44, brandGap: 64, eyebrow: 26, title: 184, titleMax: 888, sub: 40, hero: 150, unit: 44, line: 34,
    label: 26, value: 46, small: 22, footer: 24, cols: 2, colGap: 48, rowGap: 32, medal: 96, bar: 16, gap: 44, dial: { x: 950, y: 100, r: 300, w: 56 },
  },
  square: {
    pad: 88, top: 88, bottom: 88, brand: 44, brandGap: 40, eyebrow: 24, title: 150, titleMax: 640, sub: 0, hero: 112, unit: 36, line: 30,
    label: 22, value: 40, small: 22, footer: 22, cols: 3, colGap: 40, rowGap: 28, medal: 80, bar: 14, gap: 36, dial: { x: 990, y: 90, r: 220, w: 44 },
  },
  wide: {
    pad: 60, top: 52, bottom: 52, brand: 30, brandGap: 30, eyebrow: 15, title: 92, titleMax: 500, sub: 20, hero: 80, unit: 24, line: 18,
    label: 14, value: 26, small: 13, footer: 13, cols: 2, colGap: 28, rowGap: 18, medal: 44, bar: 10, gap: 26, dial: null,
  },
}
const WIDE = { w: 1200, h: 675, left: 500, rightX: 620 }

const ICON = {
  track: 'M7.05 16.95 A 7 7 0 1 1 16.95 16.95',
  arc: 'M7.05 16.95 A 7 7 0 0 1 15.5 5.94',
  spark: 'M12 8.6 Q12.5 11.5 15.4 12 Q12.5 12.5 12 15.4 Q11.5 12.5 8.6 12 Q11.5 11.5 12 8.6 Z',
}

const GLYPH: Record<Glyph, string> = {
  streak: 'M12 2.8c1.2 3 4.8 5.2 4.8 9.7a4.8 4.8 0 0 1-9.6 0c0-2.2 1-3.6 2.2-4.8.2 1.6 1 2.6 2.1 3-.5-2.9-.6-5.4.5-7.9z',
  everyday: 'M5 6.5h14a1.5 1.5 0 0 1 1.5 1.5v10.5A1.5 1.5 0 0 1 19 20H5a1.5 1.5 0 0 1-1.5-1.5V8A1.5 1.5 0 0 1 5 6.5zM3.5 10.5h17M8 4v4M16 4v4M9 15l2 2 4-4',
  tokens: 'M12 3.5 3.5 8 12 12.5 20.5 8zM3.5 12 12 16.5 20.5 12M3.5 16 12 20.5 20.5 16',
  cache: 'M13.5 2.5 5 13.5h6l-1 8 8.5-11h-6z',
  plan: 'M7 4h10l3.5 5L12 20.5 3.5 9zM3.5 9h17M9.5 4 8 9l4 11.5L16 9l-1.5-5',
  tools: 'M4 4h6.5v6.5H4zM13.5 4H20v6.5h-6.5zM4 13.5h6.5V20H4zM13.5 13.5H20V20h-6.5z',
}

export interface DrawOptions {
  format: Format
  theme: Theme
  /** `null` for the theme's own accent. */
  accent: string | null
  family: string
  screen: { w: number; h: number }
}

interface Block {
  h: number
  /** Space above the block. */
  gap: number
  draw: (x: number, y: number, w: number) => void
  /** Blocks with a higher number leave first when the card runs out of room; 0 always stays. In order: a
   * second row of stats, the tool shares, the week-by-hour grid, the subtitle and the square's note, the
   * badges, a second line under the total. */
  drop: number
  /** The first block of the group that sits low on the card; spare room goes above it. */
  anchor?: boolean
}
interface Flex {
  flex: true
  gap: number
  min: number
  ideal: number
  make: (h: number) => (x: number, y: number, w: number) => void
}
type Item = Block | Flex

/** Draws `card` onto `canvas` at full export size. */
export function drawCard(canvas: HTMLCanvasElement, card: Card, o: DrawOptions) {
  const { w: W, h: H } = sizeOf(o.format, o.screen)
  canvas.width = W
  canvas.height = H
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const c = colors(o.theme, o.accent)
  const P = painter(ctx, o.family, c)

  const bg = ctx.createLinearGradient(0, 0, W * 0.25, H)
  bg.addColorStop(0, c.bg[0])
  bg.addColorStop(1, c.bg[1])
  ctx.fillStyle = bg
  ctx.fillRect(0, 0, W, H)
  for (const g of c.glows) {
    const r = g.r * Math.max(W, H)
    const grad = ctx.createRadialGradient(g.x * W, g.y * H, 0, g.x * W, g.y * H, r)
    grad.addColorStop(0, g.color)
    grad.addColorStop(1, g.color.replace(/[\d.]+\)$/, '0)'))
    ctx.fillStyle = grad
    ctx.fillRect(0, 0, W, H)
  }

  if (o.format === 'story' || o.format === 'square') {
    const S = SPECS[o.format]
    if (S.dial && card.share !== null) dial(P, S.dial, card.share)
    portrait(P, card, S, W, H, o.format === 'story')
    return
  }
  if (o.format === 'landscape') {
    wide(P, card)
    return
  }
  // the wallpaper keeps clear of the menu bar and the Dock or taskbar
  const [safeTop, safeBottom] = [H * 0.07, H * 0.12]
  const boxW = Math.min(W * 0.84, ((H - safeTop - safeBottom) * WIDE.w) / WIDE.h)
  const scale = boxW / WIDE.w
  ctx.save()
  ctx.translate((W - boxW) / 2, safeTop + (H - safeTop - safeBottom - WIDE.h * scale) / 2)
  ctx.scale(scale, scale)
  wide(P, card)
  ctx.restore()
}

type Painter = ReturnType<typeof painter>

function painter(ctx: CanvasRenderingContext2D, family: string, c: Colors) {
  const font = (weight: number, size: number, tracking = 0) => {
    ctx.font = `${weight} ${size}px ${family}`
    if ('letterSpacing' in ctx) ctx.letterSpacing = `${(tracking * size).toFixed(2)}px`
  }
  // the baseline that centers a line of `size` text in a box of `size * lh` from `top`, as CSS does
  const base = (top: number, size: number, lh = 1.2) => top + (size * lh) / 2 + 0.36 * size
  const fit = (s: string, max: number) => {
    if (ctx.measureText(s).width <= max) return s
    let n = s.length
    while (n > 1 && ctx.measureText(s.slice(0, n) + '…').width > max) n--
    return s.slice(0, n) + '…'
  }
  /** `s` broken at spaces into at most `lines` lines of `max` width; the last one is cut short if it must be. */
  const wrap = (s: string, max: number, lines = 2) => {
    const out: string[] = []
    let cur = ''
    for (const word of s.split(' ')) {
      const next = cur ? `${cur} ${word}` : word
      if (cur && ctx.measureText(next).width > max) {
        out.push(cur)
        cur = word
      } else cur = next
    }
    out.push(cur)
    if (out.length > lines) out.splice(lines - 1, out.length, out.slice(lines - 1).join(' '))
    return out.map((l, i) => (i === out.length - 1 ? fit(l, max) : l))
  }
  const width = (items: Run[], size: number) =>
    items.reduce((sum, r) => {
      font(r.bold ? 700 : 400, size)
      return sum + ctx.measureText(r.text).width
    }, 0)
  const line = (items: Run[], x: number, y: number, size: number, ink: string, strong: string) => {
    for (const r of items) {
      font(r.bold ? 700 : 400, size)
      ctx.fillStyle = r.accent ? c.accentInk : r.bold ? strong : ink
      ctx.fillText(r.text, x, y)
      x += ctx.measureText(r.text).width
    }
    return x
  }
  const rect = (x: number, y: number, w: number, h: number, r: number, fill: string) => {
    ctx.fillStyle = fill
    ctx.beginPath()
    ctx.roundRect(x, y, w, h, Math.max(0, Math.min(r, w / 2, h / 2)))
    ctx.fill()
  }
  return { ctx, c, font, base, fit, wrap, width, line, rect }
}

function dial(P: Painter, d: { x: number; y: number; r: number; w: number }, share: number) {
  const { ctx, c } = P
  const a0 = (135 * Math.PI) / 180
  ctx.lineCap = 'round'
  ctx.lineWidth = d.w
  ctx.strokeStyle = c.track
  ctx.beginPath()
  ctx.arc(d.x, d.y, d.r, a0, a0 + 1.5 * Math.PI)
  ctx.stroke()
  if (share > 0) {
    ctx.globalAlpha = 0.9
    ctx.strokeStyle = c.accent
    ctx.beginPath()
    ctx.arc(d.x, d.y, d.r, a0, a0 + Math.min(1, share) * 1.5 * Math.PI)
    ctx.stroke()
    ctx.globalAlpha = 1
  }
}

/** Lays `items` out from `y0` to `y1`, dropping the most expendable blocks until the flexible chart has room. */
function column(items: Item[], x: number, w: number, y0: number, y1: number) {
  let list = items
  for (;;) {
    const flex = list.find((i): i is Flex => 'flex' in i)
    const fixed = list.reduce((a, i) => a + i.gap + ('flex' in i ? 0 : i.h), 0)
    const room = y1 - y0 - fixed
    const victim = list.filter((i): i is Block => !('flex' in i) && i.drop > 0).sort((a, b) => b.drop - a.drop)[0]
    if ((flex ? room < flex.min : room < 0) && victim) {
      list = list.filter((i) => i !== victim)
      continue
    }
    // with nothing left to drop, the chart takes what room there is rather than run over
    const fh = flex ? Math.max(0, Math.min(room, flex.ideal)) : 0
    const spare = Math.max(0, room - fh)
    const anchored = list.some((i) => !('flex' in i) && i.anchor)
    let y = y0
    for (const i of list) {
      if ('flex' in i) {
        y += i.gap + spare * (anchored ? 0.4 : 0.5)
        i.make(fh)(x, y, w)
        y += fh
        continue
      }
      y += i.gap + (i.anchor ? spare * (flex ? 0.6 : 1) : 0)
      i.draw(x, y, w)
      y += i.h
    }
    return
  }
}

function headerItems(P: Painter, card: Card, S: Spec, story: boolean, titleMax: number): Block[] {
  const { ctx, c } = P
  const items: Block[] = []
  items.push({
    h: S.brand, gap: 0, drop: 0,
    draw: (x, y) => {
      ctx.save()
      ctx.translate(x, y)
      ctx.scale(S.brand / 24, S.brand / 24)
      P.rect(0, 0, 24, 24, 6, '#161e3e')
      ctx.lineWidth = 2.4
      ctx.lineCap = 'round'
      ctx.strokeStyle = '#283256'
      ctx.stroke(new Path2D(ICON.track))
      ctx.strokeStyle = '#3884ff'
      ctx.stroke(new Path2D(ICON.arc))
      ctx.fillStyle = '#f2f5fa'
      ctx.fill(new Path2D(ICON.spark))
      ctx.restore()
      const size = S.brand * 0.64
      P.font(600, size, -0.01)
      ctx.fillStyle = c.ink2
      ctx.fillText('AI Usage Tracker', x + S.brand * 1.36, P.base(y, size, S.brand / size))
    },
  })
  items.push({
    h: S.eyebrow * 1.2, gap: S.brandGap, drop: 0,
    draw: (x, y, w) => {
      P.font(700, S.eyebrow, 0.16)
      ctx.fillStyle = c.accentInk
      ctx.fillText(P.fit(card.eyebrow, w * 0.8), x, P.base(y, S.eyebrow))
      if (card.pager) {
        P.font(600, S.eyebrow, 0.04)
        ctx.fillStyle = c.ink3
        ctx.textAlign = 'right'
        ctx.fillText(card.pager, x + w, P.base(y, S.eyebrow))
        ctx.textAlign = 'left'
      }
    },
  })
  // a title shrinks rather than run into the dial or past the card
  const title = story ? card.title : card.squareTitle
  let size = S.title
  P.font(800, size, -0.055)
  while (size > S.title * 0.36 && ctx.measureText(title).width > titleMax) P.font(800, (size -= 2), -0.055)
  items.push({
    h: size * 0.92, gap: 8 * (S.eyebrow / 26), drop: 0,
    draw: (x, y) => {
      P.font(800, size, -0.055)
      ctx.fillStyle = c.ink
      ctx.fillText(P.fit(title, titleMax), x, P.base(y, size, 0.92))
    },
  })
  if (S.sub && card.sub) {
    items.push({
      h: S.sub * 1.2, gap: S.sub * 0.4, drop: 3,
      draw: (x, y, w) => {
        P.font(500, S.sub, -0.015)
        ctx.fillStyle = c.ink2
        ctx.fillText(P.fit(card.sub, w), x, P.base(y, S.sub))
      },
    })
  }
  if (card.hero) {
    const hero = card.hero
    items.push({
      h: S.hero * 0.9, gap: S.gap, drop: 0,
      draw: (x, y, w) => {
        const b = P.base(y, S.hero, 0.9)
        let size = S.hero
        P.font(800, size, -0.05)
        const unitW = () => (P.font(600, S.unit), ctx.measureText(hero.unit).width)
        P.font(800, size, -0.05)
        while (size > S.hero * 0.5 && ctx.measureText(hero.value).width + S.unit * 0.5 + unitW() > w) P.font(800, (size -= 2), -0.05)
        P.font(800, size, -0.05)
        ctx.fillStyle = c.ink
        ctx.fillText(hero.value, x, b)
        const vw = ctx.measureText(hero.value).width
        P.font(600, S.unit)
        ctx.fillStyle = c.ink2
        ctx.fillText(hero.unit, x + vw + S.unit * 0.45, b)
      },
    })
  }
  card.lines.forEach((items_, i) => {
    items.push({
      h: S.line * 1.2, gap: i === 0 ? S.line * 0.7 : S.line * 0.3, drop: i === 0 ? 0 : 1,
      draw: (x, y, w) => {
        const gap = S.line * 1.2
        const lineWidth = (size: number) => items_.reduce((sum, item, k) => sum + P.width(item, size) + (k ? gap : 0), 0)
        let size = S.line
        while (size > S.line * 0.6 && lineWidth(size) > w) size -= 1
        let lx = x
        for (const item of items_) lx = P.line(item, lx, P.base(y, S.line), size, c.ink2, c.ink) + gap
      },
    })
  })
  return items
}

function statsItems(P: Painter, stats: Stat[], S: Spec, cols: number): Block[] {
  const { ctx, c } = P
  const pad = S.label * 0.75
  const cellH = pad + S.label * 1.2 + 4 + S.value * 1.2 + 4 + S.label * 1.2
  const rows: Stat[][] = []
  for (let i = 0; i < stats.length; i += cols) rows.push(stats.slice(i, i + cols))
  return rows.map((row, r) => ({
    h: cellH, gap: r === 0 ? S.gap : S.rowGap, drop: r === 0 ? 0 : 6, anchor: r === 0,
    draw: (x, y, w) => {
      const colW = (w - S.colGap * (cols - 1)) / cols
      row.forEach((s, i) => {
        const sx = x + i * (colW + S.colGap)
        let sy = y
        ctx.fillStyle = c.rule
        ctx.fillRect(sx, sy, colW, Math.max(1, S.label / 13))
        sy += pad
        P.font(400, S.label)
        ctx.fillStyle = c.ink3
        ctx.fillText(P.fit(s.label, colW), sx, P.base(sy, S.label))
        sy += S.label * 1.2 + 4
        P.font(700, S.value, -0.03)
        ctx.fillStyle = c.ink
        ctx.fillText(P.fit(s.value, colW), sx, P.base(sy, S.value))
        sy += S.value * 1.2 + 4
        // three narrow columns take the detail a size down
        const detail = cols >= 3 ? Math.min(S.label, S.small) : S.label
        P.font(400, detail)
        ctx.fillStyle = c.ink2
        ctx.fillText(P.fit(s.detail, colW), sx, P.base(sy, detail))
      })
    },
  }))
}

function toolsItem(P: Painter, tools: Share[], S: Spec, anchor: boolean): Block {
  const { ctx, c } = P
  const legend = S.small * 1.5
  return {
    h: S.bar + S.small * 0.8 + legend, gap: S.gap * 0.75, drop: 5, anchor,
    draw: (x, y, w) => {
      const gap = Math.max(2, S.bar * 0.25)
      const usable = w - gap * (tools.length - 1)
      let bx = x
      tools.forEach((s, i) => {
        const sw = Math.max(S.bar * 0.6, usable * s.share)
        P.rect(bx, y, Math.min(sw, x + w - bx), S.bar, S.bar / 2, c.toolColors[i] ?? c.ink3)
        bx += sw + gap
      })
      let lx = x
      const ly = y + S.bar + S.small * 0.8
      for (const [i, s] of tools.entries()) {
        const pct = s.pct
        P.font(600, S.small)
        const nameW = ctx.measureText(s.label).width
        P.font(700, S.small)
        const pctW = ctx.measureText(pct).width
        const itemW = S.small * 1.1 + nameW + S.small * 0.4 + pctW
        if (lx + itemW > x + w) break
        ctx.fillStyle = c.toolColors[i] ?? c.ink3
        ctx.beginPath()
        ctx.arc(lx + S.small * 0.35, P.base(ly, S.small) - S.small * 0.34, S.small * 0.32, 0, Math.PI * 2)
        ctx.fill()
        P.font(600, S.small)
        ctx.fillStyle = c.ink2
        ctx.fillText(s.label, lx + S.small * 1.1, P.base(ly, S.small))
        P.font(700, S.small)
        ctx.fillStyle = c.ink
        ctx.fillText(pct, lx + S.small * 1.5 + nameW, P.base(ly, S.small))
        lx += itemW + S.small * 1.6
      }
    },
  }
}

/** A badge in the app icon's dial: three segments for the three tiers, the reached ones in the accent,
 * around a disc tinted with it. Drawn on a 104-unit grid scaled to `d`. */
function medal(P: Painter, cx: number, cy: number, d: number, glyph: Glyph, tier: number) {
  const { ctx, c } = P
  const k = d / 104
  const ground = mix(c.bg[0], c.bg[1], 0.4)
  const disc = ctx.createRadialGradient(cx, cy - 16.3 * k, 0, cx, cy - 16.3 * k, 57.7 * k)
  disc.addColorStop(0, mix(c.accent, ground, c.light ? 0.8 : 0.72))
  disc.addColorStop(1, mix(c.accent, ground, 0.9))
  ctx.fillStyle = disc
  ctx.beginPath()
  ctx.arc(cx, cy, 37 * k, 0, Math.PI * 2)
  ctx.fill()
  const deg = Math.PI / 180
  ctx.lineWidth = 6 * k
  ctx.lineCap = 'round'
  for (let i = 0; i < 3; i++) {
    ctx.strokeStyle = i < tier ? c.accent : mix(c.track, c.ink, c.light ? 0.06 : 0.14)
    ctx.beginPath()
    ctx.arc(cx, cy, 46 * k, (138 + 90 * i) * deg, (222 + 90 * i) * deg)
    ctx.stroke()
  }
  ctx.save()
  ctx.translate(cx - 18 * k, cy - 18 * k)
  ctx.scale(1.5 * k, 1.5 * k)
  ctx.strokeStyle = c.light ? mix(c.accent, c.ink, 0.35) : c.ink
  ctx.lineWidth = 1.7
  ctx.lineJoin = 'round'
  ctx.stroke(new Path2D(GLYPH[glyph]))
  ctx.restore()
}

/** Up to three badges side by side, each centered over its title. */
function badgesRow(P: Painter, badges: Badge[], S: Spec, anchor: boolean): Block {
  const { ctx, c } = P
  const shown = badges.slice(0, 3)
  const title = S.value * 0.6
  return {
    h: S.medal * 1.2 + S.small * 0.8 + title * 1.2 + 4 + S.small * 2.4, gap: S.gap, drop: 2, anchor,
    draw: (x, y, w) => {
      const colW = w / 3
      shown.forEach((b, i) => {
        const cx = x + colW * (i + (3 - shown.length) / 2) + colW / 2
        medal(P, cx, y + S.medal * 0.6, S.medal, b.glyph, b.tier)
        let ty = y + S.medal * 1.2 + S.small * 0.8
        ctx.textAlign = 'center'
        P.font(700, title, -0.02)
        ctx.fillStyle = c.ink
        ctx.fillText(P.fit(b.title, colW - 16), cx, P.base(ty, title))
        ty += title * 1.2 + 4
        P.font(400, S.small)
        ctx.fillStyle = c.ink2
        P.wrap(b.detail, colW - 24).forEach((l, k) => ctx.fillText(l, cx, P.base(ty + k * S.small * 1.2, S.small)))
        ctx.textAlign = 'left'
      })
    },
  }
}

/** Up to three compact badges, one under the other, the medal at the left of its text. */
function badgeChips(P: Painter, badges: Badge[], S: Spec, anchor: boolean): Block {
  const { ctx, c } = P
  const shown = badges.slice(0, 3)
  const rowH = S.medal * 1.3
  return {
    h: shown.length * rowH - S.medal * 0.3, gap: S.gap, drop: 2, anchor,
    draw: (x, y, w) => {
      shown.forEach((b, i) => {
        const ry = y + i * rowH
        medal(P, x + S.medal / 2 + S.medal * 0.1, ry + S.medal / 2, S.medal, b.glyph, b.tier)
        const tx = x + S.medal * 1.45
        const tw = w - S.medal * 1.45
        const top = ry + (S.medal - S.label * 1.2 - S.small * 1.2) / 2
        P.font(700, S.label)
        ctx.fillStyle = c.ink
        ctx.fillText(P.fit(b.title, tw), tx, P.base(top, S.label))
        P.font(400, S.small)
        ctx.fillStyle = c.ink2
        ctx.fillText(P.fit(b.detail, tw), tx, P.base(top + S.label * 1.2, S.small))
      })
    },
  }
}

function noteItem(P: Painter, note: Note, S: Spec, drop: number): Block {
  const { ctx, c } = P
  const size = S.small
  return {
    h: size * 1.2, gap: size * 0.6, drop,
    draw: (x, y, w) => {
      const right = x + w
      const mid = P.base(y, size) - size * 0.34
      const sq = size * 0.8
      if (note.legend === 'heat') {
        P.font(400, size)
        ctx.fillStyle = c.ink3
        ctx.fillText(note.less, x, P.base(y, size))
        let sx = x + ctx.measureText(note.less).width + size * 0.45
        for (const col of c.heat) {
          P.rect(sx, mid - sq / 2, sq, sq, sq * 0.28, col)
          sx += sq * 1.55
        }
        P.font(400, size)
        ctx.fillStyle = c.ink3
        ctx.fillText(note.more, sx + size * 0.1, P.base(y, size))
      } else if (note.left.length) {
        P.rect(x, mid - sq / 2, sq, sq, sq * 0.28, c.accent)
        P.line(note.left, x + sq + size * 0.55, P.base(y, size), size, c.ink3, c.ink)
      }
      const rw = P.width(note.right, size)
      P.line(note.right, right - rw, P.base(y, size), size, c.ink3, c.ink)
      if (note.avgLine) dash(ctx, right - rw - size * 1.8, right - rw - size * 0.55, mid, c.ink3, Math.max(1, size / 11))
    },
  }
}

/** A chart's ideal and smallest height at width `w`, and how it draws at a given height. */
function chartItem(P: Painter, chart: Chart, S: Spec, format: 'story' | 'square' | 'wide', w: number, gap: number): Flex {
  const { ctx, c } = P
  const small = S.small
  const tickH = small * 1.2 + small * 0.7
  const flex = (min: number, ideal: number, make: Flex['make']): Flex => ({ flex: true, gap, min: Math.min(min, ideal), ideal, make })

  if (chart.kind === 'bars') {
    const ideal = format === 'story' ? 256 : format === 'square' ? 162 : 170
    return flex(ideal * 0.55, ideal, (h) => (x, y, w) => {
      const n = chart.values.length
      const slot = w / n
      const bw = Math.round(slot * (n <= 12 ? 0.56 : 0.64))
      const max = Math.max(...chart.values, 1)
      const peak = chart.values.indexOf(Math.max(...chart.values))
      const barMax = Math.max(small, h - tickH - small * 0.4)
      const floor = y + small * 0.4 + barMax
      chart.values.forEach((v, i) => {
        // a day or hour without use keeps a short stub so the axis reads whole
        const bh = v > 0 ? Math.max(small * 0.3, (v / max) * barMax) : small * 0.18
        P.rect(x + i * slot + (slot - bw) / 2, floor - bh, bw, bh, Math.min(small * 0.3, bw / 3), v <= 0 ? c.track : i === peak ? c.accent : c.bar)
      })
      if (chart.avg !== null) dash(ctx, x, x + w, floor - (chart.avg / max) * barMax, c.ink3, Math.max(1, small / 11))
      P.font(400, small)
      ctx.fillStyle = c.ink3
      ctx.textAlign = 'center'
      for (const tk of chart.ticks) ctx.fillText(tk.label, x + tk.i * slot + slot / 2, P.base(floor + small * 0.7, small))
      ctx.textAlign = 'left'
    })
  }

  if (chart.kind === 'strip') {
    const cols = Math.ceil((chart.levels.length + chart.offset) / 7)
    const pitch = Math.min(small * 0.77, (w + small * 0.13) / cols)
    const h = tickH + 7 * pitch
    return flex(h, h, () => (x, y) => {
      const cell = pitch * 0.82
      P.font(400, small)
      ctx.fillStyle = c.ink3
      for (const tk of chart.ticks) ctx.fillText(tk.label, x + tk.col * pitch, P.base(y, small))
      const gy = y + tickH
      chart.levels.forEach((lvl, i) => {
        const at = i + chart.offset
        P.rect(x + Math.floor(at / 7) * pitch, gy + (at % 7) * pitch, cell, cell, cell * 0.25, c.heat[lvl])
      })
    })
  }

  if (chart.kind === 'month') {
    const rows = Math.ceil((chart.offset + chart.levels.length) / 7)
    const head = small * 1.9
    const ideal = head + rows * (w / 7)
    return flex(head + rows * (w / 7) * 0.5, ideal, (h) => (x, y, w) => {
      // the days fill the width; with too little height they turn into wide rectangles, as on a wall calendar
      const px = w / 7
      const py = Math.min(px, (h - head) / rows)
      const gap = px * 0.12
      const [cw, ch] = [px - gap, py - gap]
      const r = Math.min(cw, ch) * 0.24
      P.font(600, small)
      ctx.fillStyle = c.ink3
      ctx.textAlign = 'center'
      chart.weekdays.forEach((d, i) => ctx.fillText(d, x + i * px + cw / 2, P.base(y, small)))
      const gy = y + head
      chart.levels.forEach((lvl, i) => {
        const at = i + chart.offset
        const cx = x + (at % 7) * px
        const cy = gy + Math.floor(at / 7) * py
        if (lvl < 0) {
          ctx.strokeStyle = c.rule
          ctx.lineWidth = Math.max(1, cw * 0.016)
          ctx.beginPath()
          ctx.roundRect(cx + 0.5, cy + 0.5, cw - 1, ch - 1, r)
          ctx.stroke()
        } else P.rect(cx, cy, cw, ch, r, c.heat[lvl])
        if (chart.today === i) {
          const o = gap * 0.3
          ctx.strokeStyle = c.accentInk
          ctx.lineWidth = Math.max(2, cw * 0.035)
          ctx.beginPath()
          ctx.roundRect(cx - o, cy - o, cw + 2 * o, ch + 2 * o, r + o)
          ctx.stroke()
        }
        const size = Math.min(cw, ch) * 0.34
        P.font(600, size)
        ctx.fillStyle = lvl >= 4 ? c.onAccent : lvl < 0 ? mix(c.ink3, c.bg[0], 0.4) : lvl >= 1 ? c.ink : c.ink3
        ctx.fillText(String(i + 1), cx + cw / 2, P.base(cy + ch / 2 - size * 0.6, size))
      })
      ctx.textAlign = 'left'
    })
  }

  if (chart.kind === 'year') {
    const cols = 4
    const gapM = small * 1.6
    const monthW = (w - gapM * (cols - 1)) / cols
    const head = small * 1.7
    const rows = 12 / cols
    const heightAt = (pitch: number) => rows * (head + 6 * pitch) + (rows - 1) * gapM
    const ideal = heightAt(monthW / 7)
    return flex(heightAt((monthW / 7) * 0.55), ideal, (h) => (x, y, w) => {
      const pitch = Math.min((w - gapM * (cols - 1)) / cols / 7, (h - (rows - 1) * gapM - rows * head) / rows / 6)
      const mw = pitch * 7
      const span = mw * cols + gapM * (cols - 1)
      const x0 = x + (w - span) / 2
      const cell = pitch * 0.8
      chart.months.forEach((m, k) => {
        const mx = x0 + (k % cols) * (mw + gapM)
        const my = y + Math.floor(k / cols) * (head + 6 * pitch + gapM)
        P.font(600, small)
        ctx.fillStyle = c.ink2
        ctx.fillText(m.name, mx, P.base(my, small))
        m.levels.forEach((lvl, i) => {
          const at = i + m.offset
          const cx = mx + (at % 7) * pitch
          const cy = my + head + Math.floor(at / 7) * pitch
          if (lvl < 0) {
            ctx.strokeStyle = c.rule
            ctx.lineWidth = Math.max(1, cell * 0.06)
            ctx.beginPath()
            ctx.roundRect(cx + 0.5, cy + 0.5, cell - 1, cell - 1, cell * 0.25)
            ctx.stroke()
          } else P.rect(cx, cy, cell, cell, cell * 0.25, c.heat[lvl])
        })
      })
    })
  }

  if (chart.kind === 'clock') {
    const ideal = Math.min(w, format === 'story' ? 760 : format === 'square' ? 470 : 380)
    return flex(ideal * 0.6, ideal, (h) => (x, y, w) => {
      const size = Math.min(w, h)
      const cx = x + w / 2
      const cy = y + size / 2
      const R = size / 2
      const outer = R - small * 2.1
      const inner = outer * 0.5
      const step = (Math.PI * 2) / 24
      const angle = (hr: number) => -Math.PI / 2 + hr * step
      if (chart.window !== null) {
        ctx.fillStyle = mix(c.empty, c.accent, 0.12)
        ctx.beginPath()
        ctx.arc(cx, cy, outer + small * 0.6, angle(chart.window), angle(chart.window + 6))
        ctx.arc(cx, cy, inner - small * 0.6, angle(chart.window + 6), angle(chart.window), true)
        ctx.closePath()
        ctx.fill()
      }
      ctx.strokeStyle = c.rule
      ctx.lineWidth = Math.max(1, small / 14)
      ctx.beginPath()
      ctx.arc(cx, cy, inner - small * 0.6, 0, Math.PI * 2)
      ctx.stroke()
      const max = Math.max(...chart.values, 1)
      const peak = chart.values.indexOf(Math.max(...chart.values))
      const lw = ((Math.PI * 2 * inner) / 24) * 0.62
      ctx.lineCap = 'round'
      chart.values.forEach((v, hr) => {
        const a = angle(hr + 0.5)
        const len = v > 0 ? Math.max(lw * 0.2, (v / max) * (outer - inner - lw)) : 0
        const [ux, uy] = [Math.cos(a), Math.sin(a)]
        ctx.strokeStyle = v <= 0 ? c.track : hr === peak ? c.accent : c.bar
        ctx.lineWidth = v <= 0 ? lw * 0.5 : lw
        ctx.beginPath()
        ctx.moveTo(cx + ux * (inner + lw / 2), cy + uy * (inner + lw / 2))
        ctx.lineTo(cx + ux * (inner + lw / 2 + len), cy + uy * (inner + lw / 2 + len))
        ctx.stroke()
      })
      P.font(600, small)
      ctx.fillStyle = c.ink3
      ctx.textAlign = 'center'
      for (const hr of [0, 6, 12, 18]) {
        const a = angle(hr)
        ctx.fillText(String(hr).padStart(2, '0'), cx + Math.cos(a) * (R - small * 0.9), P.base(cy + Math.sin(a) * (R - small * 0.9) - small * 0.6, small))
      }
      const big = inner * 0.36
      P.font(800, big, -0.04)
      ctx.fillStyle = c.ink
      ctx.fillText(chart.center, cx, P.base(cy - big * 0.75, big))
      P.font(500, small)
      ctx.fillStyle = c.ink3
      ctx.fillText(P.fit(chart.caption, inner * 1.5), cx, P.base(cy + big * 0.55, small))
      ctx.textAlign = 'left'
    })
  }

  if (chart.kind === 'punch') {
    const labelW = small * 3
    const pitch = (w - labelW) / 24
    const h = 7 * pitch + tickH
    return flex(h, h, () => (x, y, w) => {
      const p = (w - labelW) / 24
      const all = chart.values.flat()
      const max = Math.max(...all, 1)
      const levels = heatLevels(all)
      chart.weekdays.forEach((d, row) => {
        P.font(500, small)
        ctx.fillStyle = c.ink3
        ctx.fillText(d, x, P.base(y + row * p + p / 2 - small * 0.6, small))
        for (let hr = 0; hr < 24; hr++) {
          const v = chart.values[row]?.[hr] ?? 0
          const r = v > 0 ? (0.25 + 0.75 * Math.sqrt(v / max)) * p * 0.42 : p * 0.07
          ctx.fillStyle = v > 0 ? c.heat[Math.max(1, levels[row * 24 + hr])] : c.track
          ctx.beginPath()
          ctx.arc(x + labelW + hr * p + p / 2, y + row * p + p / 2, r, 0, Math.PI * 2)
          ctx.fill()
        }
      })
      P.font(400, small)
      ctx.fillStyle = c.ink3
      ctx.textAlign = 'center'
      for (const hr of [0, 6, 12, 18]) ctx.fillText(String(hr).padStart(2, '0'), x + labelW + hr * p + p / 2, P.base(y + 7 * p + small * 0.5, small))
      ctx.textAlign = 'left'
    })
  }

  if (chart.kind === 'ranking') {
    const label = S.value * 0.78
    const rowH = label * 1.2 + small * 0.6 + S.bar + small * 1.6
    const h = chart.rows.length * rowH
    // spare room spreads the rows apart rather than gathering under them
    return flex(h, h * 1.45, (fh) => (x, y, w) => {
      const pitch = Math.max(rowH, fh / chart.rows.length)
      const top = Math.max(...chart.rows.map((r) => r.share), 1e-9)
      chart.rows.forEach((r, i) => {
        const ry = y + i * pitch
        P.font(700, label, -0.02)
        const vw = ctx.measureText(r.value).width
        ctx.fillStyle = c.ink
        ctx.textAlign = 'right'
        ctx.fillText(r.value, x + w, P.base(ry, label))
        ctx.textAlign = 'left'
        P.font(i === 0 ? 700 : 600, label, -0.02)
        ctx.fillStyle = i === 0 ? c.ink : c.ink2
        ctx.fillText(P.fit(r.label, w - vw - label), x, P.base(ry, label))
        const by = ry + label * 1.2 + small * 0.6
        P.rect(x, by, w, S.bar, S.bar / 2, c.track)
        P.rect(x, by, Math.max(S.bar, (w * r.share) / top), S.bar, S.bar / 2, i === 0 ? c.accent : c.bar)
      })
    })
  }

  // a year's badges, two to a row, spread over the room there is
  const rows = Math.ceil(chart.badges.length / 2)
  const d = S.medal * 1.1
  const cellH = d * 1.4
  const h = rows * cellH + (rows - 1) * S.rowGap
  return flex(h, h * 1.5, (fh) => (x, y, w) => {
    const pitch = Math.max(cellH + S.rowGap, (fh + S.rowGap) / rows)
    const colW = (w - S.colGap) / 2
    const title = S.value * 0.64
    chart.badges.forEach((b, i) => {
      const bx = x + (i % 2) * (colW + S.colGap)
      const mid = y + Math.floor(i / 2) * pitch + cellH / 2
      medal(P, bx + d / 2 + d * 0.15, mid, d, b.glyph, b.tier)
      const tx = bx + d * 1.4
      const tw = colW - d * 1.4
      P.font(400, S.small)
      const lines = P.wrap(b.detail, tw)
      const top = mid - (title * 1.2 + 6 + lines.length * S.small * 1.2) / 2
      P.font(700, title, -0.02)
      ctx.fillStyle = c.ink
      ctx.fillText(P.fit(b.title, tw), tx, P.base(top, title))
      P.font(400, S.small)
      ctx.fillStyle = c.ink2
      lines.forEach((l, k) => ctx.fillText(l, tx, P.base(top + title * 1.2 + 6 + k * S.small * 1.2, S.small)))
    })
  })
}

function heatLevels(values: number[]): number[] {
  const used = values.filter((v) => v > 0).sort((a, b) => a - b)
  const q = [0.25, 0.5, 0.75].map((p) => used[Math.floor(p * (used.length - 1))] ?? 0)
  return values.map((v) => (v > 0 ? 1 + q.filter((x) => v > x).length : 0))
}

function footer(P: Painter, S: Spec, x: number, y: number) {
  P.font(400, S.footer)
  P.ctx.fillStyle = P.c.ink3
  P.ctx.fillText('github.com/efekoca/ai-usage-tracker', x, P.base(y, S.footer))
}

function portrait(P: Painter, card: Card, S: Spec, W: number, H: number, story: boolean) {
  const w = W - 2 * S.pad
  const items: Item[] = headerItems(P, card, S, story, S.titleMax)
  // a square has no height to spare for a calendar or the clock, so the stats stand beside it
  if (!story && card.chart && ['month', 'year', 'clock'].includes(card.chart.kind)) {
    const chartW = w * 0.62
    const statX = chartW + S.colGap * 1.5
    const inner = chartItem(P, card.chart, S, 'square', chartW, S.gap)
    const stats = statsItems(P, card.square, S, 1)
    const statsH = stats.reduce((a, b) => a + b.h, 0)
    items.push({
      flex: true, gap: S.gap, min: Math.max(inner.min, statsH), ideal: Math.max(inner.ideal, statsH),
      make: (h) => (x, y) => {
        inner.make(h)(x, y, chartW)
        const pitch = stats.length > 1 ? (h - stats[0].h) / (stats.length - 1) : 0
        stats.forEach((b, i) => b.draw(x + statX, y + i * Math.min(pitch, b.h * 1.35), w - statX))
      },
    })
    const footerY = H - S.bottom - S.footer * 1.2
    column(items, S.pad, w, S.top, footerY - S.gap * 0.6)
    footer(P, S, S.pad, footerY)
    return
  }
  if (card.chart) {
    items.push(chartItem(P, card.chart, S, story ? 'story' : 'square', w, S.gap))
    if (card.note) items.push(noteItem(P, card.note, S, story ? 0 : 3))
  }
  if (story && card.extra) items.push({ ...chartBlock(P, card.extra, S, 'story', w), drop: 4 })
  const stats = story ? card.stats : card.square
  const statBlocks = statsItems(P, stats, S, story && stats.length === 3 ? 3 : S.cols)
  items.push(...statBlocks)
  if (card.tools.length) items.push(toolsItem(P, card.tools, S, !statBlocks.length))
  if (card.badges.length) items.push(badgesRow(P, card.badges, S, !statBlocks.length && !card.tools.length))
  const footerY = H - S.bottom - S.footer * 1.2
  column(items, S.pad, w, S.top, footerY - S.gap * 0.6)
  footer(P, S, S.pad, footerY)
}

/** A fixed-height block of a chart that does not stretch. */
function chartBlock(P: Painter, chart: Chart, S: Spec, format: 'story' | 'square' | 'wide', w: number): Block {
  const f = chartItem(P, chart, S, format, w, S.gap)
  return { h: f.ideal, gap: f.gap, drop: 0, draw: f.make(f.ideal) }
}

function wide(P: Painter, card: Card) {
  const S = SPECS.wide
  const { w: W, h: H, left, rightX } = WIDE
  const rightW = W - rightX - S.pad
  const footerY = H - S.bottom - S.footer * 1.2
  const bottom = footerY - S.gap * 0.6

  const leftItems: Item[] = headerItems(P, card, S, true, left)
  if (card.badges.length) leftItems.push(badgeChips(P, card.badges, S, true))
  column(leftItems, S.pad, left, S.top, bottom)
  footer(P, S, S.pad, footerY)

  const rightItems: Item[] = []
  if (card.chart) {
    rightItems.push(chartItem(P, card.chart, S, 'wide', rightW, 0))
    if (card.note) rightItems.push(noteItem(P, card.note, S, 3))
  }
  const statBlocks = statsItems(P, card.stats.slice(0, 4), S, 2)
  rightItems.push(...statBlocks)
  if (card.tools.length) rightItems.push(toolsItem(P, card.tools, S, !statBlocks.length))
  column(rightItems, rightX, rightW, S.top + S.brand * 0.2, H - S.bottom)
}

function dash(ctx: CanvasRenderingContext2D, x0: number, x1: number, y: number, color: string, width = 2) {
  ctx.save()
  ctx.strokeStyle = color
  ctx.globalAlpha = 0.7
  ctx.lineWidth = width
  ctx.lineCap = 'butt'
  ctx.setLineDash([width * 3, width * 4])
  ctx.beginPath()
  ctx.moveTo(x0, y)
  ctx.lineTo(x1, y)
  ctx.stroke()
  ctx.restore()
}
