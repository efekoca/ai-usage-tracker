// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ExperimentSide, MarkerView, Settings } from '../lib/api'

const mocks = vi.hoisted(() => ({
  experiments: vi.fn(),
  projects: vi.fn(),
  projectsForSettings: vi.fn(),
  saveSettings: vi.fn(),
}))
vi.mock('../lib/api', () => ({ api: mocks, on: () => Promise.resolve(() => {}) }))

const { app } = await import('../lib/store.svelte')
const { default: Experiments } = await import('./Experiments.svelte')

const day = 86_400_000
const at = new Date(2026, 10, 4).getTime()
const side = (over: Partial<ExperimentSide> = {}): ExperimentSide => ({ from_ms: at - 7 * day, to_ms: at, sessions: 11, requests: 341, cost_usd: 31.46, unpriced_requests: 0, cost_per_session: 2.86, started: 11, starting_load: 38_400, tool_calls: 240, tool_failed: 22, tool_error_pct: 9.2, cache_hit_pct: 91.4, ...over })
const after = (over: Partial<ExperimentSide> = {}) => side({ from_ms: at, to_ms: at + 7 * day, sessions: 12, requests: 372, cost_per_session: 2.21, started: 12, starting_load: 29_500, tool_calls: 265, tool_failed: 15, tool_error_pct: 5.7, cache_hit_pct: 95.8, ...over })
const marker = (over: Partial<MarkerView> = {}): MarkerView => ({ id: 'm1', at_ms: at, label: 'Shorter CLAUDE.md', project_id: 3, project: 'notes', project_hidden: false, project_gone: false, tool: 'claude_code', days: 7, before: side(), after: after(), ...over })

let done: (() => void)[] = []
beforeEach(() => {
  app.settings = { markers: [] } as unknown as Settings
  mocks.projects.mockResolvedValue([{ id: 3, path: '', name: '', hidden: true }])
  mocks.projectsForSettings.mockResolvedValue([{ id: 3, path: '/w/notes', name: 'notes', hidden: true }])
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

async function render(rows: MarkerView[]) {
  mocks.experiments.mockResolvedValue(rows)
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(Experiments, { target })
  await settle()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}

const cell = (row: Element, name: string) => row.querySelector(name)?.textContent?.trim() ?? ''
const figures = (card: Element) => [...card.querySelectorAll('.row')].map((r) => [cell(r, '.name'), `${cell(r, '.before')} → ${cell(r, 'b.after')}`, cell(r, '.delta')].filter(Boolean).join(' | '))
const widths = (card: Element) => [...card.querySelectorAll('.row')].map((r) => [...r.querySelectorAll<HTMLElement>('.bar')].map((b) => Math.round(parseFloat(b.style.width))))
const tones = (card: Element) => [...card.querySelectorAll('.change')].map((c) => (c.classList.contains('good') ? 'good' : c.classList.contains('bad') ? 'bad' : ''))

describe('before and after a marker', () => {
  it('shows each figure before and after, and calls a clear change better or worse', async () => {
    const target = await render([marker()])
    const card = target.querySelector('.marker')!
    expect(card.querySelector('.what')?.textContent?.replace(/\s+/g, ' ').trim()).toBe('Shorter CLAUDE.md Wednesday, November 4, 2026 · notes · Claude Code · 7 days on each side')
    expect(figures(card)).toEqual([
      'Cost per session | $2.86 → $2.21 | −23% better',
      'Starting load | 38.4K → 29.5K | −23% better',
      'Tool error rate | 9.2% → 5.7% | −3.5 pp better',
      'Cache hit rate | 91.4% → 95.8% | +4.4 pp better',
    ])
    expect(tones(card)).toEqual(['good', 'good', 'good', 'good'])
    expect(card.querySelector('.basis')?.textContent?.replace(/\s+/g, ' ').trim()).toBe('Oct 28 – Nov 3: 11 sessions, 341 requests · Nov 4 – Nov 10: 12 sessions, 372 requests')
    expect(card.querySelector('.thin')).toBeNull()
    // each pair of bars is drawn against the larger of the two
    expect(widths(card)).toEqual([[100, 77], [100, 77], [100, 62], [95, 100]])

    const worse = await render([marker({ after: after({ cost_per_session: 4.1, starting_load: 39_000, tool_error_pct: 14.8, cache_hit_pct: 91.0 }) })])
    expect(tones(worse.querySelector('.marker')!)).toEqual(['bad', '', 'bad', ''])
    expect(figures(worse.querySelector('.marker')!)[1]).toBe('Starting load | 38.4K → 39K | +1.6%')
  })

  it('calls nothing better or worse on thin data, and says so', async () => {
    const few = after({ to_ms: at + 2 * day, sessions: 2, requests: 62, started: 2, tool_calls: 61 })
    const target = await render([marker({ project_id: null, project: '', tool: null, after: few })])
    const card = target.querySelector('.marker')!
    expect(tones(card)).toEqual(['', '', '', ''])
    expect(card.querySelector('.thin')?.textContent).toContain('fewer than three sessions')
    expect(card.querySelector('.basis')?.textContent).toContain('Nov 4 – Nov 10: 2 sessions, 62 requests')
    expect(card.querySelector('.what .subtle')?.textContent?.replace(/\s+/g, ' ').trim()).toBe('Wednesday, November 4, 2026 · 7 days on each side · 2 of 7 days have passed')
    // enough sessions, but too few tool calls to judge the error rate
    const calls = await render([marker({ after: after({ tool_calls: 12, tool_error_pct: 25 }) })])
    expect(tones(calls.querySelector('.marker')!)).toEqual(['good', 'good', '', 'good'])
  })

  it('shows what is not known as such', async () => {
    const empty = after({ to_ms: at, sessions: 0, requests: 0, cost_per_session: null, started: 0, starting_load: null, tool_calls: 0, tool_error_pct: null, cache_hit_pct: null })
    const target = await render([marker({ project: '', project_gone: true, after: empty }), marker({ id: 'm2', project: '', project_hidden: true })])
    const [ahead, hidden] = target.querySelectorAll('.marker')
    expect(figures(ahead)[0]).toBe('Cost per session | $2.86 → —')
    expect(ahead.querySelector('.basis')?.textContent).toContain('Nov 4 – Nov 10: not started yet')
    expect(widths(ahead)[0]).toEqual([])
    expect(ahead.querySelector('.thin')).toBeNull()
    expect(ahead.querySelector('.what .subtle')?.textContent).toContain('Project not in the archive')
    expect(hidden.querySelector('.what .subtle')?.textContent).toContain('Hidden project #3')
  })

  it('leaves the cost uncoloured while a model has no price, and takes no more than it keeps', async () => {
    const target = await render([marker({ after: after({ unpriced_requests: 40 }) })])
    const card = target.querySelector('.marker')!
    expect(tones(card)).toEqual(['', 'good', 'good', 'good'])
    expect(card.textContent).toContain('40 requests of a model without a price are in no cost')
    // a few of them among hundreds change little: the cost is still weighed
    const few = await render([marker({ before: side({ unpriced_requests: 9 }), after: after({ unpriced_requests: 4 }) })])
    expect(tones(few.querySelector('.marker')!)).toEqual(['good', 'good', 'good', 'good'])
    expect(few.querySelector('.marker')!.textContent).not.toContain('without a price')

    app.settings = { markers: Array.from({ length: 50 }, (_, i) => ({ id: `m${i}` })) } as unknown as Settings
    const full = await render([])
    const label = full.querySelector<HTMLInputElement>('form input')!
    label.value = 'One more'
    label.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(full.querySelector<HTMLButtonElement>('button[type=submit]')!.disabled).toBe(true)
    expect(full.querySelector('.under')?.textContent?.trim()).toBe('Only 50 can be kept. Remove a marker to add another.')
  })

  it('adds a marker with its day, scope and days, and removes one', async () => {
    const target = await render([])
    expect(target.querySelector('.none')?.textContent).toContain('No marker yet')
    // the bar says which days a marker set today would compare
    expect(target.querySelector('.chip.day')?.textContent?.trim()).toBe('Today')
    expect(target.querySelector('.under')?.textContent).toMatch(/will be compared with .* · Press Enter to add/)
    const add = target.querySelector<HTMLButtonElement>('button[type=submit]')!
    expect(add.disabled).toBe(true)
    const [label, date] = target.querySelectorAll<HTMLInputElement>('form input')
    label.value = '  Added an MCP server '
    label.dispatchEvent(new Event('input', { bubbles: true }))
    date.value = '2026-11-02'
    date.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(add.disabled).toBe(false)
    add.click()
    await settle()
    const saved = (mocks.saveSettings.mock.lastCall?.[0] as Settings).markers
    expect(saved).toHaveLength(1)
    expect(saved[0]).toMatchObject({ at_ms: new Date(2026, 10, 2).getTime(), label: 'Added an MCP server', project_id: null, project_path: null, tool: null, days: 7 })
    expect(saved[0].id).toBeTruthy()
    expect(label.value).toBe('')
    expect(target.querySelector('.chip.day')?.textContent?.trim()).toBe('Nov 2')
    expect(target.querySelector('.under')?.textContent?.replace(/\s+/g, ' ').trim()).toBe('Oct 26 – Nov 1 will be compared with Nov 2 – Nov 8 · Press Enter to add')

    // a second Enter while the first is still being saved adds nothing more
    let saveNow: (s: Settings) => void = () => {}
    mocks.saveSettings.mockImplementationOnce((next: Settings) => new Promise<Settings>((done) => (saveNow = () => done(next))))
    const calls = mocks.saveSettings.mock.calls.length
    label.value = 'Twice'
    label.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    const form = target.querySelector('form')!
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await settle()
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await settle()
    expect(mocks.saveSettings.mock.calls.length).toBe(calls + 1)
    expect(add.disabled).toBe(true)
    saveNow(app.settings as Settings)
    await settle()

    // a day that has not begun, or no day, is not a marker
    const type = (value: string, day: string) => {
      label.value = value
      label.dispatchEvent(new Event('input', { bubbles: true }))
      date.value = day
      date.dispatchEvent(new Event('input', { bubbles: true }))
      flushSync()
    }
    type('later', '2999-01-01')
    expect(add.disabled).toBe(true)
    type('later', '')
    expect(add.disabled).toBe(true)

    app.settings = { markers: [{ id: 'm1' }, { id: 'm2' }] } as unknown as Settings
    const listed = await render([marker(), marker({ id: 'm2', label: 'Second' })])
    listed.querySelector<HTMLButtonElement>('.marker button')!.click()
    await settle()
    expect((mocks.saveSettings.mock.lastCall?.[0] as Settings).markers.map((m) => m.id)).toEqual(['m2'])
  })
})
