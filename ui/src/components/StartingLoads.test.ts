// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { StartingLoad } from '../lib/api'

const mocks = vi.hoisted(() => ({ startingLoads: vi.fn() }))
vi.mock('../lib/api', () => ({ api: mocks, on: () => Promise.resolve(() => {}) }))

const { default: StartingLoads } = await import('./StartingLoads.svelte')

const day = 86_400_000
const t0 = new Date(2026, 10, 1, 9).getTime()
function load(over: Partial<StartingLoad>, sizes: number[]): StartingLoad {
  const starts = sizes.map((tokens, i) => ({ ts_ms: t0 + i * day, tokens }))
  return { project_id: 1, project: 'notes', hidden: false, tool: 'claude_code', client: 'cli', conversations: sizes.length, typical: sizes[sizes.length - 1], smallest: Math.min(...sizes), cost_usd: 0.25, change: null, starts, ...over }
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

async function render(loads: StartingLoad[]) {
  mocks.startingLoads.mockResolvedValue(loads)
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(StartingLoads, { target })
  await settle()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}

const rows = (t: HTMLElement) => [...t.querySelectorAll('tbody tr')].map((r) => [...r.children].map((c) => c.textContent?.replace(/\s+/g, ' ').trim()).filter(Boolean).join(' '))

describe('the starting load of each project', () => {
  it('shows nothing without a conversation that began in the period', async () => {
    const target = await render([])
    expect(target.querySelector('section')).toBeNull()
  })

  it('lists each project with what a conversation begins with and since when it changed', async () => {
    const grown = load({ change: { at_ms: t0 + 3 * day, before: 38_000, after: 50_200 }, typical: 50_200 }, [38_000, 38_400, 37_900, 50_200, 50_600, 49_900])
    const shrunk = load({ project_id: 2, project: 'site', client: 'claude-desktop', change: { at_ms: t0 + 2 * day, before: 50_000, after: 30_100 } }, [50_000, 50_500, 30_000, 30_400, 30_100])
    const hidden = load({ project_id: 7, project: '', hidden: true, tool: 'codex', client: null, cost_usd: null }, [19_000])
    const target = await render([grown, shrunk, hidden])
    const [a, b, c] = rows(target)
    expect(a).toBe('notes Claude Code · Terminal 6 50.2K 37.9K $0.25 +12.2K Nov 4')
    expect(b).toBe('site Claude Code · Claude desktop 5 30.1K 30K $0.25 −19.9K Nov 3')
    expect(c).toBe('Hidden project #7 Codex 1 19K 19K — —')
    const steps = [...target.querySelectorAll('.step')]
    expect(steps.map((s) => s.classList.contains('up'))).toEqual([true, false])
    expect(steps[0].getAttribute('title')).toContain('50.2K tokens instead of 38K')
    // the line marks where the change began; one conversation is a single point
    const sparks = [...target.querySelectorAll('.spark')]
    expect(sparks.map((s) => [!!s.querySelector('.since'), !!s.querySelector('polyline')])).toEqual([[true, true], [true, true], [false, false]])
    expect(sparks[0].querySelector('polyline')?.getAttribute('points')?.split(' ')).toHaveLength(6)
    expect(sparks[0].getAttribute('aria-label')).toBe('First requests of 6 conversations: the first 38K, the latest 49.9K tokens')
  })

  it('tells the conversation under the pointer', async () => {
    const target = await render([load({}, [38_000, 38_400, 37_900, 50_200, 50_600, 49_900])])
    const spark = target.querySelector('.spark')!
    const move = (x: number) => {
      spark.dispatchEvent(new PointerEvent('pointermove', { clientX: x, bubbles: true }))
      flushSync()
      return target.querySelector('.tip')?.textContent?.replace(/\s+/g, ' ').trim()
    }
    expect(target.querySelector('.tip')).toBeNull()
    // the same short figure as the table's
    expect(move(0)).toMatch(/^38K Nov 1, /)
    expect(move(80)).toMatch(/^50\.2K Nov 4, /)
    expect(move(500)).toMatch(/^49\.9K Nov 6, /)
    expect(target.querySelectorAll('.at')).toHaveLength(1)
    spark.dispatchEvent(new PointerEvent('pointerleave'))
    flushSync()
    expect(target.querySelector('.tip')).toBeNull()
  })

  it('keeps a wobble of a few hundred tokens a flat line', async () => {
    const target = await render([load({}, [75_000, 75_300, 74_900])])
    const ys = target.querySelector('polyline')!.getAttribute('points')!.split(' ').map((p) => Number(p.split(',')[1]))
    expect(Math.max(...ys) - Math.min(...ys)).toBeLessThan(2)
  })

  it('shows the first ten and the rest on request', async () => {
    const target = await render(Array.from({ length: 13 }, (_, i) => load({ project_id: i, project: `p${i}` }, [40_000])))
    expect(rows(target)).toHaveLength(10)
    const more = target.querySelector<HTMLButtonElement>('.more')!
    expect(more.textContent).toBe('Show all (13)')
    more.click()
    flushSync()
    expect(rows(target)).toHaveLength(13)
    expect(target.querySelector('.more')).toBeNull()
  })
})
