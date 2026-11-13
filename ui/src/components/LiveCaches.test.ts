// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { LiveCache, RefreshInfo, Settings } from '../lib/api'

const mocks = vi.hoisted(() => ({
  handlers: {} as Record<string, () => void>,
  liveCaches: vi.fn(),
  cacheRefreshInfo: vi.fn(),
  refreshCache: vi.fn(),
  setCacheAuto: vi.fn(),
  dropCache: vi.fn(),
}))
vi.mock('../lib/api', () => ({
  api: mocks,
  on: (event: string, fn: () => void) => {
    mocks.handlers[event] = fn
    return Promise.resolve(() => {})
  },
}))

const { app } = await import('../lib/store.svelte')
const { default: LiveCaches } = await import('./LiveCaches.svelte')

const now = Date.now()
const cache = (over: Partial<LiveCache>): LiveCache => ({ session_id: 's', project: 'notes', branch: 'main', model: 'claude-opus-5-5', client: 'cli', ttl_ms: 3_600_000, expires_ms: now + 1_800_000, started_ms: now - 3_600_000, tokens: 1000, refresh_usd: 0.1, rewrite_usd: 1, refreshable: true, refreshing: false, refresh_failed: false, unasked: false, auto: false, auto_capped: false, refreshes: 0, refreshes_usd: 0, ...over })

let done: (() => void)[] = []
beforeEach(() => {
  app.settings = { cache_timer: true, cache_lead: 2 } as unknown as Settings
  mocks.cacheRefreshInfo.mockResolvedValue({ installed: true, settings_file: '', today: 0, today_usd: 0 } satisfies RefreshInfo)
})
afterEach(() => {
  done.forEach((d) => d())
  done = []
  vi.clearAllMocks()
})

const settle = async () => {
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0))
  flushSync()
}

async function render(caches: LiveCache[]) {
  mocks.liveCaches.mockResolvedValue(caches)
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(LiveCaches, { target })
  await settle()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  const row = (id: string) => [...target.querySelectorAll('li')].find((li) => li.querySelector('.name')?.textContent === id)!
  return { target, row }
}

describe('the Live caches list', () => {
  it('says what happens to each conversation', async () => {
    const { row } = await render([
      cache({ session_id: 'a', project: 'auto', auto: true }),
      cache({ session_id: 'b', project: 'failed', refresh_failed: true }),
      cache({ session_id: 'c', project: 'unasked', refreshable: false, unasked: true }),
    ])
    expect(row('auto').querySelector('.status')?.textContent).toMatch(/Refreshes at/)
    expect(row('failed').querySelector('.status')?.textContent).toContain('Not refreshed')
    expect(row('unasked').querySelector('.status')?.textContent).toMatch(/Runs out at/)
    expect(row('unasked').querySelector<HTMLButtonElement>('.go')?.title).toContain('bypassPermissions')
    expect(row('unasked').querySelector<HTMLButtonElement>('.go')?.disabled).toBe(true)
  })

  it('shows why a refresh could not be asked for', async () => {
    mocks.refreshCache.mockRejectedValue('not_waiting')
    const { row } = await render([cache({ project: 'notes' })])
    row('notes').querySelector<HTMLButtonElement>('.go')!.click()
    await settle()
    expect(mocks.refreshCache).toHaveBeenCalledWith('s')
    expect(row('notes').querySelector('.status.bad')?.textContent).toContain("Claude Code isn't waiting")
  })

  it('moves the focus through the row menu from the keyboard', async () => {
    const { row } = await render([cache({ project: 'notes' })])
    const trigger = row('notes').querySelector<HTMLButtonElement>('.menu-wrap > button')!
    trigger.click()
    flushSync()
    const items = [...row('notes').querySelectorAll<HTMLElement>('[role^="menuitem"]')]
    expect(document.activeElement).toBe(items[0])
    const key = (k: string) => document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }))
    key('ArrowDown')
    expect(document.activeElement).toBe(items[1])
    key('ArrowDown')
    expect(document.activeElement).toBe(items[0])
    key('Escape')
    flushSync()
    expect(row('notes').querySelector('[role="menu"]')).toBeNull()
    expect(document.activeElement).toBe(trigger)
  })

  it('does not bring back a row the user stopped following', async () => {
    let answer: (v: LiveCache[]) => void = () => {}
    const { target, row } = await render([cache({ project: 'notes' })])
    // a reload already on its way still holds the row
    mocks.liveCaches.mockReturnValue(new Promise((r) => (answer = r)))
    mocks.dropCache.mockResolvedValue(undefined)
    mocks.handlers['data-changed']()
    row('notes').querySelector<HTMLButtonElement>('.menu-wrap > button')!.click()
    flushSync()
    ;[...row('notes').querySelectorAll<HTMLElement>('[role="menuitem"]')].at(-1)!.click()
    flushSync()
    answer([cache({ project: 'notes' })])
    await settle()
    expect(target.querySelector('li')).toBeNull()
  })
})
