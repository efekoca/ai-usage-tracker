// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { TurnCost, Turns as Report } from '../lib/api'

const mocks = vi.hoisted(() => ({ turnCosts: vi.fn() }))
vi.mock('../lib/api', () => ({ api: mocks, on: () => Promise.resolve(() => {}) }))

const { default: Turns } = await import('./Turns.svelte')
const { app } = await import('../lib/store.svelte')

const t0 = new Date(2026, 10, 12, 14, 32).getTime()
function turn(over: Partial<TurnCost>): TurnCost {
  return {
    id: 'cc:a', tool: 'claude_code', client: 'cli', session_id: 's1', ordinal: 3, origin: 'human', project_id: 1, project: 'notes', hidden: false,
    started_ms: t0, ended_ms: t0 + 18 * 60_000, requests: 84, agents: 3, tool_calls: 120, tool_failed: 2, calls_hidden: false, total_tokens: 4_200_000, cost_usd: 11.4, unpriced_requests: 0,
    models: ['claude-opus-5-5'],
    parts: [
      { agent: null, threads: 0, requests: 20, cost_usd: 2.85 },
      { agent: 'Explore', threads: 3, requests: 64, cost_usd: 8.55 },
    ],
    ...over,
  }
}

let done: (() => void)[] = []
afterEach(() => {
  done.forEach((d) => d())
  done = []
  vi.clearAllMocks()
})

const settle = async () => {
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0))
  flushSync()
}

async function render(report: Partial<Report>) {
  mocks.turnCosts.mockResolvedValue({ turns: [], tracked: true, requests_outside: 0, ...report })
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(Turns, { target })
  await settle()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}

const text = (e: Element | null | undefined) => e?.textContent?.replace(/\s+/g, ' ').trim() ?? ''
const rows = (t: HTMLElement) => [...t.querySelectorAll('tbody tr:not(.detail)')].map((r) => [...r.children].map(text).filter(Boolean).join(' | '))

describe('what each prompt cost', () => {
  it('shows nothing until a log names its turns', async () => {
    const target = await render({ tracked: false })
    expect(target.querySelector('section')).toBeNull()
  })

  it('says so when no turn began in the period', async () => {
    const target = await render({})
    expect(text(target.querySelector('p.muted'))).toBe('No turn began in this period.')
    expect(target.querySelector('table')).toBeNull()
  })

  it('gathers a turn into one line, the costliest first', async () => {
    const cheap = turn({ id: 'cc:b', started_ms: t0 + 3_600_000, ended_ms: t0 + 3_600_000 + 40_000, ordinal: 4, requests: 2, agents: 0, tool_calls: 0, tool_failed: 0, cost_usd: 0.31, parts: [{ agent: null, threads: 0, requests: 2, cost_usd: 0.31 }] })
    const target = await render({ turns: [cheap, turn({})] })
    expect(rows(target)).toEqual([
      'Nov 12, 02:32 PM notes · Turn 3 | 84 | 3 | 120 2 failed | 18 min | $11.40',
      'Nov 12, 03:32 PM notes · Turn 4 | 2 | — | — | < 1 min | $0.31',
    ])
    // three quarters of the first turn's cost came from its subagents
    const bars = [...target.querySelectorAll<HTMLElement>('.split i')].map((b) => Math.round(parseFloat(b.style.width)))
    expect(bars).toEqual([75])

    const newest = [...target.querySelectorAll('button')].find((b) => text(b) === 'Newest')
    newest?.click()
    flushSync()
    expect(rows(target).map((r) => r.slice(0, 16))).toEqual(['Nov 12, 03:32 PM', 'Nov 12, 02:32 PM'])
  })

  it('labels a turn the user did not type and keeps a hidden project unnamed', async () => {
    const target = await render({
      turns: [
        turn({ id: 'cc:n', origin: 'task_notification', agents: 0, cost_usd: 3 }),
        turn({ id: 'cc:x', origin: 'something_new', project: '', hidden: true, project_id: 7, agents: 0, cost_usd: 2 }),
        turn({ id: 'cx:s', tool: 'codex', origin: 'sdk', project: '', project_id: null, agents: 0, cost_usd: 1 }),
      ],
    })
    expect(rows(target).map((r) => r.split(' | ')[0].replace('Nov 12, 02:32 PM ', ''))).toEqual(['notes · Turn 3 Background task', 'Hidden project #7 · Turn 3 Automatic', 'No project · Turn 3 Program'])
  })

  it('opens a turn to its parts and its session', async () => {
    const target = await render({ turns: [turn({ unpriced_requests: 4 })] })
    target.querySelector<HTMLButtonElement>('button[aria-expanded]')?.click()
    flushSync()
    expect([...target.querySelectorAll('.parts li')].map(text)).toEqual(['Main conversation 20 requests $2.85', 'Explore × 3 64 requests $8.55'])
    expect(text(target.querySelector('.unpriced'))).toBe('4 requests have no known price and are left out of the cost.')
    app.view = 'overview'
    ;[...target.querySelectorAll('button')].find((b) => text(b) === 'Open session')?.click()
    expect([app.view, app.openSession]).toEqual(['sessions', 'claude_codes1'])
  })

  const facts = (t: HTMLElement) => [...t.querySelectorAll('.facts div')].map((d) => [...d.children].map(text).join(' '))

  it('shows no cost for a turn whose every request lacks a price, in its row or in the summary', async () => {
    const unknown = turn({ cost_usd: 0, unpriced_requests: 84, agents: 0 })
    const target = await render({ turns: [unknown] })
    expect(rows(target)[0].endsWith('| —')).toBe(true)
    expect(facts(target)).toEqual(['Turns 1', 'Typical turn cost —', 'Share of the costliest 10% —'])
    // such turns do not pull the typical cost of the others down
    const mixed = await render({ turns: [unknown, { ...unknown, id: 'cc:u2' }, turn({ id: 'cc:p', cost_usd: 6 })] })
    expect(facts(mixed)).toEqual(['Turns 3', 'Typical turn cost $6.00', 'Share of the costliest 10% —'])
  })

  it('leaves the tool calls out of a turn the filter splits', async () => {
    const target = await render({ turns: [turn({ calls_hidden: true, tool_calls: 0, tool_failed: 0 })] })
    expect(rows(target)[0]).toBe('Nov 12, 02:32 PM notes · Turn 3 | 84 | 3 | — | 18 min | $11.40')
    expect(target.querySelector('tbody td[title]')?.getAttribute('title')).toContain('the tool calls are not counted')
  })

  it('shows five turns first and says how many requests belong to none', async () => {
    const many = Array.from({ length: 8 }, (_, i) => turn({ id: `cc:${i}`, cost_usd: 8 - i }))
    const target = await render({ turns: many, requests_outside: 214 })
    expect(rows(target)).toHaveLength(5)
    const more = [...target.querySelectorAll('button')].find((b) => text(b).startsWith('Show more'))
    expect(text(more)).toBe('Show more (3 left)')
    more?.click()
    flushSync()
    expect(rows(target)).toHaveLength(8)
    expect(text(target.querySelector('.note'))).toContain('214 requests whose log is deleted or names no turn are not listed.')
    expect([...target.querySelectorAll('.facts div')].map((d) => [...d.children].map(text).join(' '))).toEqual(['Turns 8', 'Typical turn cost $4.00', 'Share of the costliest 10% —'])
  })
})
