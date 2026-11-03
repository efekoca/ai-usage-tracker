import type { Card, Run, Stat } from './cards'

export type Shape = 'story' | 'square'
export type Theme = 'dark' | 'light'

export const SIZE: Record<Shape, { w: number; h: number }> = { story: { w: 1080, h: 1920 }, square: { w: 1080, h: 1080 } }

const COLORS = {
  dark: {
    bg: '#0b1020', ink: '#ffffff', ink2: '#b8c0d6', ink3: '#8a93ab', accent: '#3884ff', track: '#1a2244', rule: 'rgba(255,255,255,0.1)',
    heat: ['#161d36', '#1f3466', '#28509e', '#3071dc', '#5b9bff'], bar: '#28509e', peak: '#5b9bff',
  },
  light: {
    bg: '#f4f5f8', ink: '#0e1220', ink2: '#3f4657', ink3: '#5f6576', accent: '#2f6feb', track: '#dde2ee', rule: 'rgba(14,18,32,0.1)',
    heat: ['#e3e7f0', '#c4d4f7', '#8fb0f2', '#4f84ee', '#2f6feb'], bar: '#8fb0f2', peak: '#2f6feb',
  },
}

// Instagram covers the top and bottom of a story with its own controls, so the story keeps 200px clear
const LAYOUT = {
  story: {
    pad: 96, top: 200, bottom: 200, dial: { x: 950, y: 100, r: 300, w: 56 }, brandGap: 72, title: 220, titleMax: 888, sub: 40,
    heroGap: 64, hero: 150, unit: 44, unitGap: 20, lineGap: 24, line: 34, itemGap: 40, chartGap: 64, cell: 14, rowPitch: 18,
    barTop: 12, barMax: 200, tickBase: 248, barBlock: 256, statsGap: 64, cols: 2, colGap: 48, rowGap: 32, label: 26, value: 46, footer: 24,
  },
  square: {
    pad: 88, top: 88, bottom: 88, dial: { x: 990, y: 90, r: 220, w: 44 }, brandGap: 40, title: 160, titleMax: 640, sub: 0,
    heroGap: 40, hero: 112, unit: 36, unitGap: 16, lineGap: 16, line: 30, itemGap: 32, chartGap: 40, cell: 14, rowPitch: 18,
    barTop: 6, barMax: 118, tickBase: 158, barBlock: 162, statsGap: 40, cols: 3, colGap: 40, rowGap: 32, label: 22, value: 40, footer: 22,
  },
}

const ICON = {
  track: 'M7.05 16.95 A 7 7 0 1 1 16.95 16.95',
  arc: 'M7.05 16.95 A 7 7 0 0 1 15.5 5.94',
  spark: 'M12 8.6 Q12.5 11.5 15.4 12 Q12.5 12.5 12 15.4 Q11.5 12.5 8.6 12 Q11.5 11.5 12 8.6 Z',
}

/** Draws `card` onto `canvas` at full export size. */
export function drawCard(canvas: HTMLCanvasElement, card: Card, shape: Shape, theme: Theme, family: string) {
  const { w: W, h: H } = SIZE[shape]
  canvas.width = W
  canvas.height = H
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const L = LAYOUT[shape]
  const c = COLORS[theme]
  const story = shape === 'story'
  const right = W - L.pad

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
  const line = (items: Run[], x: number, y: number, size: number, ink: string, strong: string) => {
    for (const r of items) {
      font(r.bold ? 700 : 400, size)
      ctx.fillStyle = r.bold ? strong : ink
      ctx.fillText(r.text, x, y)
      x += ctx.measureText(r.text).width
    }
    return x
  }
  const width = (items: Run[], size: number) =>
    items.reduce((sum, r) => {
      font(r.bold ? 700 : 400, size)
      return sum + ctx.measureText(r.text).width
    }, 0)

  ctx.textBaseline = 'alphabetic'
  ctx.textAlign = 'left'
  ctx.fillStyle = c.bg
  ctx.fillRect(0, 0, W, H)

  // the app icon's dial, filled to the share of active days or hours
  const { dial } = L
  const a0 = (135 * Math.PI) / 180
  ctx.lineCap = 'round'
  ctx.lineWidth = dial.w
  ctx.strokeStyle = c.track
  ctx.beginPath()
  ctx.arc(dial.x, dial.y, dial.r, a0, a0 + 1.5 * Math.PI)
  ctx.stroke()
  if (card.share > 0) {
    ctx.globalAlpha = 0.9
    ctx.strokeStyle = c.accent
    ctx.beginPath()
    ctx.arc(dial.x, dial.y, dial.r, a0, a0 + card.share * 1.5 * Math.PI)
    ctx.stroke()
    ctx.globalAlpha = 1
  }

  let y = L.top
  ctx.save()
  ctx.translate(L.pad, y)
  ctx.scale(44 / 24, 44 / 24)
  ctx.fillStyle = '#161e3e'
  ctx.beginPath()
  ctx.roundRect(0, 0, 24, 24, 6)
  ctx.fill()
  ctx.lineWidth = 2.4
  ctx.strokeStyle = '#283256'
  ctx.stroke(new Path2D(ICON.track))
  ctx.strokeStyle = '#3884ff'
  ctx.stroke(new Path2D(ICON.arc))
  ctx.fillStyle = '#f2f5fa'
  ctx.fill(new Path2D(ICON.spark))
  ctx.restore()
  font(600, 28, -0.01)
  ctx.fillStyle = c.ink2
  ctx.fillText('AI Usage Tracker', L.pad + 60, base(y, 28, 44 / 28))
  y += 44 + L.brandGap

  font(700, 26, 0.16)
  ctx.fillStyle = c.accent
  ctx.fillText(card.eyebrow, L.pad, base(y, 26))
  y += 26 * 1.2 + 8

  // a title shrinks rather than run into the dial or past the card
  const title = story ? card.title : card.squareTitle
  let size = L.title
  font(800, size, -0.055)
  while (size > 72 && ctx.measureText(title).width > L.titleMax) font(800, (size -= 4), -0.055)
  ctx.fillStyle = c.ink
  ctx.fillText(title, L.pad, base(y, L.title, 0.92))
  y += L.title * 0.92

  if (story) {
    y += 16
    font(500, L.sub, -0.015)
    ctx.fillStyle = c.ink2
    ctx.fillText(fit(card.sub, W - 2 * L.pad), L.pad, base(y, L.sub))
    y += L.sub * 1.2
  }

  y += L.heroGap
  const heroBase = base(y, L.hero, 0.9)
  font(800, L.hero, -0.05)
  ctx.fillStyle = c.ink
  ctx.fillText(card.hero, L.pad, heroBase)
  const heroW = ctx.measureText(card.hero).width
  font(600, L.unit)
  ctx.fillStyle = c.ink2
  ctx.fillText(card.unit, L.pad + heroW + L.unitGap, heroBase)
  y += L.hero * 0.9

  y += L.lineGap
  // a long amount in another currency shrinks the line rather than run past the margin
  const lineWidth = (size: number) => card.line.reduce((sum, item, i) => sum + width(item, size) + (i ? L.itemGap : 0), 0)
  let lineSize = L.line
  while (lineSize > 20 && lineWidth(lineSize) > W - 2 * L.pad) lineSize -= 1
  let x = L.pad
  for (const item of card.line) x = line(item, x, base(y, L.line), lineSize, c.ink2, c.ink) + L.itemGap
  y += L.line * 1.2

  y += L.chartGap
  const chart = card.chart
  if (chart.kind === 'heat') {
    const cols = Math.ceil((chart.levels.length + chart.offset) / 7)
    const pitch = Math.min(16.8, (W - 2 * L.pad + 2.8) / cols)
    const cell = Math.min(L.cell, pitch - 2.4)
    font(400, 22)
    ctx.fillStyle = c.ink3
    for (const tk of chart.ticks) ctx.fillText(tk.label, L.pad + tk.col * pitch, base(y, 22))
    y += 26 + 12
    chart.levels.forEach((lvl, i) => {
      const at = i + chart.offset
      ctx.fillStyle = c.heat[lvl]
      ctx.beginPath()
      ctx.roundRect(L.pad + Math.floor(at / 7) * pitch, y + (at % 7) * L.rowPitch, cell, cell, 3.5)
      ctx.fill()
    })
    y += 7 * L.rowPitch - 2
    if (story) {
      y += 12
      const mid = y + 13
      font(400, 22)
      ctx.fillStyle = c.ink3
      ctx.fillText(chart.less, L.pad, base(y, 22))
      let sx = L.pad + ctx.measureText(chart.less).width + 10
      for (const col of c.heat) {
        ctx.fillStyle = col
        ctx.beginPath()
        ctx.roundRect(sx, mid - 9, 18, 18, 5)
        ctx.fill()
        sx += 28
      }
      ctx.fillStyle = c.ink3
      font(400, 22)
      ctx.fillText(chart.more, sx + 2, base(y, 22))
      line(card.right, right - width(card.right, 22), base(y, 22), 22, c.ink3, c.ink)
      y += 26
    }
  } else {
    const n = chart.values.length
    const slot = (W - 2 * L.pad) / n
    const bw = Math.round(slot * 0.64)
    const max = Math.max(...chart.values, 1)
    const peak = chart.values.indexOf(Math.max(...chart.values))
    const floor = y + L.barTop + L.barMax
    chart.values.forEach((v, i) => {
      // a day or hour without use keeps a short stub so the axis reads whole
      const h = v > 0 ? Math.max(6, (v / max) * L.barMax) : 4
      ctx.fillStyle = v <= 0 ? c.track : i === peak ? c.peak : c.bar
      ctx.beginPath()
      ctx.roundRect(L.pad + i * slot + (slot - bw) / 2, floor - h, bw, h, Math.min(6, bw / 3, h / 2))
      ctx.fill()
    })
    if (chart.avg !== null) {
      const ay = floor - (chart.avg / max) * L.barMax
      dash(ctx, L.pad, right, ay, c.ink3)
    }
    font(400, 22)
    ctx.fillStyle = c.ink3
    ctx.textAlign = 'center'
    for (const tk of chart.ticks) ctx.fillText(tk.label, L.pad + tk.i * slot + slot / 2, y + L.tickBase)
    ctx.textAlign = 'left'
    y += L.barBlock
    if (story) {
      y += 12
      const mid = y + 13
      ctx.fillStyle = c.peak
      ctx.beginPath()
      ctx.roundRect(L.pad, mid - 9, 18, 18, 5)
      ctx.fill()
      line(card.left, L.pad + 30, base(y, 22), 22, c.ink3, c.ink)
      const rw = width(card.right, 22)
      line(card.right, right - rw, base(y, 22), 22, c.ink3, c.ink)
      if (card.avgLine) dash(ctx, right - rw - 40, right - rw - 12, mid, c.ink3)
      y += 26
    }
  }

  y += L.statsGap
  const stats: Stat[] = story ? card.stats : card.square
  const colW = (W - 2 * L.pad - L.colGap * (L.cols - 1)) / L.cols
  const cellH = 20 + L.label * 1.2 + 4 + L.value * 1.2 + 4 + L.label * 1.2
  stats.forEach((s, i) => {
    const sx = L.pad + (i % L.cols) * (colW + L.colGap)
    let sy = y + Math.floor(i / L.cols) * (cellH + L.rowGap)
    ctx.fillStyle = c.rule
    ctx.fillRect(sx, sy, colW, 2)
    sy += 20
    font(400, L.label)
    ctx.fillStyle = c.ink3
    ctx.fillText(fit(s.label, colW), sx, base(sy, L.label))
    sy += L.label * 1.2 + 4
    font(700, L.value, -0.03)
    ctx.fillStyle = c.ink
    ctx.fillText(fit(s.value, colW), sx, base(sy, L.value))
    sy += L.value * 1.2 + 4
    font(400, L.label)
    ctx.fillStyle = c.ink2
    ctx.fillText(fit(s.detail, colW), sx, base(sy, L.label))
  })

  font(400, L.footer)
  ctx.fillStyle = c.ink3
  ctx.fillText('github.com/efekoca/ai-usage-tracker', L.pad, base(H - L.bottom - L.footer * 1.2, L.footer))
}

function dash(ctx: CanvasRenderingContext2D, x0: number, x1: number, y: number, color: string) {
  ctx.save()
  ctx.strokeStyle = color
  ctx.globalAlpha = 0.7
  ctx.lineWidth = 2
  ctx.lineCap = 'butt'
  ctx.setLineDash([6, 8])
  ctx.beginPath()
  ctx.moveTo(x0, y)
  ctx.lineTo(x1, y)
  ctx.stroke()
  ctx.restore()
}
