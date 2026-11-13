// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { CacheMisses as Misses, MissEvent, MissProject, Settings } from '../lib/api'

const mocks = vi.hoisted(() => ({ cacheMisses: vi.fn() }))
vi.mock('../lib/api', async (actual) => ({
  ...(await actual<typeof import('../lib/api')>()),
  api: mocks,
  on: () => Promise.resolve(() => {}),
}))

const { app } = await import('../lib/store.svelte')
const { default: CacheMisses } = await import('./CacheMisses.svelte')

const misses = (over: Partial<Misses>): Misses => ({ reported: true, sessions: 4, misses: 9, rebuilds: 2, recached_tokens: 1_840_000, causes: [], undetermined: 0, days: [], projects: [], ...over })
const event = (over: Partial<MissEvent>): MissEvent => ({ ts_ms: Date.UTC(2026, 10, 13, 9, 30), session_id: 's1', branch: 'main', model: 'claude-opus-5-5', misses: 1, rebuilds: 0, recached_tokens: 64_000, causes: ['ttl_expired_5m'], ...over })
const project = (over: Partial<MissProject>): MissProject => ({ project_id: 1, project: 'notes', hidden: false, misses: 1, rebuilds: 0, recached_tokens: 64_000, last_ms: Date.UTC(2026, 10, 13, 9, 30), cause: 'ttl_expired_5m', events: [event({})], more: 0, ...over })

let done: (() => void)[] = []
beforeEach(() => {
  app.settings = { enabled_sources: ['claude_code'], capture: { statusline: false } } as unknown as Settings
})
afterEach(() => {
  done.forEach((d) => d())
  done = []
  vi.clearAllMocks()
})

async function render(data: Misses) {
  mocks.cacheMisses.mockResolvedValue(data)
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(CacheMisses, { target })
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0))
  flushSync()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}

describe('the cache misses card', () => {
  it('shows the misses by cause, with an explanation for each cause it knows', async () => {
    const t = await render(misses({ causes: [{ cause: 'ttl_expired_5m', misses: 5 }, { cause: 'tools_changed', misses: 3 }, { cause: 'model_switched', misses: 1 }], undetermined: 1 }))
    expect([...t.querySelectorAll('.tile .value')].map((v) => v.textContent)).toEqual(['9', '1.8M', '2'])
    expect([...t.querySelectorAll('.legend .cname')].map((n) => n.textContent)).toEqual(['The 5-minute lifetime ran out', 'Tool definitions changed', 'Model switched', 'Cause not found'])
    const tip = t.querySelector('.legend tr:first-child [role="tooltip"]')!
    expect(tip.textContent).toContain('5 minutes')
    expect(t.querySelector('.legend tr:first-child .tip-btn')?.getAttribute('aria-describedby')).toBe(tip.id)
    expect(t.querySelector('.legend tr:nth-child(3) .tip'), 'no explanation for a cause it does not know').toBeNull()
  })

  it('lists projects, the main cause of each, and opens one into its misses', async () => {
    const t = await render(
      misses({
        projects: [
          project({ misses: 3, events: [event({}), event({ misses: 0, rebuilds: 1, causes: [], recached_tokens: 0 }), event({ session_id: 's3', causes: ['tools_changed', 'ttl_expired_5m'], misses: 2, branch: 'a-very-long-branch-name' })] }),
          project({ project_id: 2, project: '', hidden: true, misses: 2, cause: null }),
          project({ project_id: 5, project: 'site', misses: 0, rebuilds: 1, cause: null }),
        ],
      }),
    )
    const groups = [...t.querySelectorAll('tr.group')]
    expect(groups.map((g) => [g.querySelector('.pname')?.textContent, g.querySelector('.count')?.textContent?.trim(), g.querySelector('.why')?.textContent?.trim()])).toEqual([
      ['notes', '3', 'The 5-minute lifetime ran out'],
      ['Hidden project #2', '2', 'Cause not found'],
      ['site', '—', 'Rebuilt after a compaction or clearing'],
    ])
    expect(t.querySelector('tr.detail')).toBeNull()

    const toggle = groups[0].querySelector<HTMLButtonElement>('.toggle-col button')!
    toggle.click()
    flushSync()
    expect(toggle.getAttribute('aria-expanded')).toBe('true')
    const rows = [...t.querySelectorAll('tr.detail .events li')]
    expect(rows.map((r) => r.querySelector('.what')?.textContent)).toEqual(['The 5-minute lifetime ran out', 'Rebuilt after a compaction or clearing', '2 misses · Tool definitions changed, The 5-minute lifetime ran out'])
    expect(rows[0].querySelector('.where .model')?.textContent).toBe('Opus 5.5')
    expect(rows[2].querySelector<HTMLElement>('.chip')?.title).toBe('a-very-long-branch-name')
    expect(rows[1].querySelector('.dot.hollow')).not.toBeNull()
    rows[2].querySelector<HTMLButtonElement>('button')!.click()
    expect([app.view, app.openSession]).toEqual(['sessions', 'claude_codes3'])
  })

  it('opens a project from anywhere on its row, once per click, and says when older misses are left out', async () => {
    const t = await render(misses({ projects: [project({ more: 12 }), project({ project_id: 2, project: 'site' })] }))
    const [first] = [...t.querySelectorAll<HTMLElement>('tr.group')]
    first.querySelector<HTMLElement>('.pname')!.click()
    flushSync()
    expect(t.querySelectorAll('tr.detail').length).toBe(1)
    expect(t.querySelector('.events .older')?.textContent).toBe('12 older entries are not shown here.')
    // the arrow is inside the row: its click must not open and close at once
    first.querySelector<HTMLButtonElement>('.toggle-col button')!.click()
    flushSync()
    expect(t.querySelector('tr.detail')).toBeNull()
  })

  it('asks for the status-line bridge until Claude Code has reported anything', async () => {
    const t = await render(misses({ reported: false, misses: 0, rebuilds: 0 }))
    expect(t.querySelector('.setup')?.textContent).toContain('turn on the Claude Code status-line bridge')
    t.querySelector<HTMLButtonElement>('.setup button')!.click()
    expect(app.view).toBe('sources')
  })

  it('says when a period has none, and stays away without Claude Code', async () => {
    expect((await render(misses({ misses: 0, rebuilds: 0 }))).querySelector('.empty')?.textContent).toContain('no cache misses in this period')
    app.settings = { enabled_sources: ['codex'], capture: { statusline: true } } as unknown as Settings
    expect((await render(misses({}))).querySelector('section')).toBeNull()
  })
})
