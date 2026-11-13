// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { OpenerInfo, OpenerRun, Settings } from '../lib/api'

const mocks = vi.hoisted(() => ({
  windowOpenerInfo: vi.fn(),
  openWindowNow: vi.fn(),
  saveSettings: vi.fn(),
}))
vi.mock('../lib/api', () => ({ api: mocks, on: () => Promise.resolve(() => {}) }))

const { app } = await import('../lib/store.svelte')
const { default: WindowOpener } = await import('./WindowOpener.svelte')

const now = Date.now()
const run = (over: Partial<OpenerRun> = {}): OpenerRun => ({ at_ms: now - 60_000, day: '2026-11-13', outcome: 'sent', resets_ms: now + 5 * 3_600_000, tokens: 1313, error: null, ...over })
const info = (over: Partial<OpenerInfo> = {}): OpenerInfo => ({ available: true, running: false, next_ms: now + 3_600_000, runs: [], ...over })
const settings = (o: Partial<Settings['window_opener']> = {}) => ({ window_opener: { enabled: true, minute: 360, days: [1, 2, 3, 4, 5], ...o } }) as unknown as Settings

let done: (() => void)[] = []
beforeEach(() => {
  app.settings = settings()
  // as the app does: what was saved comes back as the settings
  mocks.saveSettings.mockImplementation(async (next: Settings) => next)
})
afterEach(() => {
  done.forEach((d) => d())
  done = []
  vi.clearAllMocks()
})

const settle = async () => {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0))
  flushSync()
}

async function render(first: OpenerInfo) {
  mocks.windowOpenerInfo.mockResolvedValue(first)
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(WindowOpener, { target })
  await settle()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}

const days = (t: HTMLElement) => [...t.querySelectorAll<HTMLButtonElement>('.day')]
const pressed = (t: HTMLElement) => days(t).map((d) => (d.getAttribute('aria-pressed') === 'true' ? 1 : 0)).join('')
const saved = () => (mocks.saveSettings.mock.lastCall?.[0] as Settings).window_opener

describe('the window scheduler', () => {
  it('draws the two windows on the day, also past midnight', async () => {
    const target = await render(info())
    const near = (css: string) => Math.round(parseFloat(css) * 100) / 100
    const bands = (t: HTMLElement) => [...t.querySelectorAll<HTMLElement>('.band')].map((b) => [near(b.style.left), near(b.style.width), b.textContent])
    const marks = (t: HTMLElement) => [...t.querySelectorAll('.scale > *')].map((m) => m.textContent)
    expect(bands(target)).toEqual([
      [25, 20.83, 'Window 1'],
      [45.83, 20.83, 'Window 2'],
    ])
    expect(marks(target)).toEqual(['00:00', '06:00opens', '11:00resets', '24:00'])
    expect(target.querySelector('.tag')?.textContent).toMatch(/^Next: /)

    app.settings = settings({ minute: 22 * 60 + 30 })
    const late = await render(info())
    expect(bands(late).map((b) => b[0])).toEqual([93.75, 0, 14.58])
    expect(marks(late)).toEqual(['00:00', '22:30opens', '03:30resets'])
  })

  it('shows only what it does and what it costs while it is off', async () => {
    app.settings = settings({ enabled: false })
    const target = await render(info({ next_ms: null }))
    expect(target.querySelector('input[type=time]')).toBeNull()
    expect(target.textContent).toContain('1,000 to 1,500 tokens')
    target.querySelector<HTMLButtonElement>('button[role=switch]')!.click()
    await settle()
    expect(saved().enabled).toBe(true)
    expect(target.querySelector('input[type=time]')).not.toBeNull()
  })

  it('saves the time and the days, and keeps one day on', async () => {
    const target = await render(info())
    const time = target.querySelector<HTMLInputElement>('input[type=time]')!
    expect(time.value).toBe('06:00')
    expect(pressed(target)).toBe('1111100')
    time.value = '07:35'
    time.dispatchEvent(new Event('change', { bubbles: true }))
    await settle()
    expect(saved().minute).toBe(7 * 60 + 35)
    days(target)[6].click()
    await settle()
    expect(saved().days).toEqual([1, 2, 3, 4, 5, 7])
    for (const i of [0, 1, 2, 3, 4]) {
      days(target)[i].click()
      await settle()
    }
    expect(pressed(target)).toBe('0000001')
    const calls = mocks.saveSettings.mock.calls.length
    days(target)[6].click()
    await settle()
    expect(mocks.saveSettings.mock.calls.length).toBe(calls)
    expect(pressed(target)).toBe('0000001')
  })

  it('tells what each run did', async () => {
    const target = await render(info({ runs: [run(), run({ at_ms: now - 2, outcome: 'open', tokens: null }), run({ at_ms: now - 3, outcome: 'failed', resets_ms: null, tokens: null, error: 'claude_limit_full' }), run({ at_ms: now - 4 })] }))
    const rows = [...target.querySelectorAll('.run')].map((r) => r.textContent?.replace(/\s+/g, ' ').trim())
    expect(rows).toHaveLength(3)
    expect(rows[0]).toMatch(/Request sent · 1,313 tokens Window resets at \d/)
    expect(rows[1]).toMatch(/A window was already open · no request sent Window resets at \d/)
    expect(rows[2]).toContain('Could not open it: the limit is full')
    const past = await render(info({ runs: [run({ resets_ms: now - 1000 })] }))
    expect(past.querySelector('.run')?.textContent).toMatch(/Window reset at \d/)
  })

  it('sends a request now, and not without Claude Code', async () => {
    const target = await render(info())
    const button = () => target.querySelector<HTMLButtonElement>('.btn.try')!
    mocks.openWindowNow.mockResolvedValue(info({ runs: [run({ day: null })] }))
    button().click()
    await settle()
    expect(mocks.openWindowNow).toHaveBeenCalledOnce()
    expect(target.querySelectorAll('.run')).toHaveLength(1)

    mocks.openWindowNow.mockRejectedValueOnce('running')
    button().click()
    await settle()
    expect(target.querySelector('[role=alert]')?.textContent).toContain('running')

    const none = await render(info({ available: false }))
    expect(none.querySelector<HTMLButtonElement>('.btn.try')!.disabled).toBe(true)
    expect(none.textContent).toContain('Claude Code was not found')
  })
})
