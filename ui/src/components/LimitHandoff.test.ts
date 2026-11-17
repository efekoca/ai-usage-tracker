// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { HandoffInfo, Settings } from '../lib/api'

const mocks = vi.hoisted(() => ({
  handoffInfo: vi.fn(),
  setLimitHandoff: vi.fn(),
  openHandoffNote: vi.fn(),
  continueHandoffInCodex: vi.fn(),
}))
vi.mock('../lib/api', () => ({ api: mocks, on: () => Promise.resolve(() => {}) }))

const { app } = await import('../lib/store.svelte')
const { default: LimitHandoff } = await import('./LimitHandoff.svelte')

const folder = '/Users/you/Library/Application Support/ai-usage-tracker/handoff/notes'
const info = (over: Partial<HandoffInfo> = {}): HandoffInfo => ({
  installed: true,
  settings_file: '/Users/you/.claude/settings.json',
  folder,
  codex_found: true,
  notes: [{ key: 'site-1a2b3c4d', project: 'site', written_ms: Date.now() - 60_000 }],
  ...over,
})

let done: (() => void)[] = []
beforeEach(() => {
  app.settings = { limit_handoff: true } as unknown as Settings
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

async function render(first: HandoffInfo) {
  mocks.handoffInfo.mockResolvedValue(first)
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(LimitHandoff, { target })
  await settle()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}

const toggle = (t: HTMLElement) => t.querySelector<HTMLButtonElement>('button[role=switch]')!
const button = (t: HTMLElement, label: string) => t.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!

describe('the handoff note', () => {
  it('names the hook, the settings file and the one folder it may write to before adding them', async () => {
    app.settings = { limit_handoff: false } as unknown as Settings
    const target = await render(info({ installed: false, notes: [] }))
    toggle(target).click()
    flushSync()
    const ask = target.querySelector('.ask')!
    expect(ask.textContent).toContain('/Users/you/.claude/settings.json')
    expect(ask.textContent).toContain(folder)
    expect(mocks.setLimitHandoff).not.toHaveBeenCalled()
    mocks.setLimitHandoff.mockResolvedValue(info({ notes: [] }))
    ;[...ask.querySelectorAll('button')].at(-1)!.click()
    await settle()
    expect(mocks.setLimitHandoff).toHaveBeenCalledWith(true)
    expect(target.querySelector('.ask')).toBeNull()
  })

  it('opens a note or continues it in Codex, and says why when Codex is missing', async () => {
    const target = await render(info())
    expect(target.querySelector('.row')?.textContent).toContain('site')
    button(target, 'Open the note: site').click()
    await settle()
    expect(mocks.openHandoffNote).toHaveBeenCalledWith('site-1a2b3c4d')
    mocks.continueHandoffInCodex.mockRejectedValue('terminal_failed')
    button(target, 'Continue in Codex: site').click()
    await settle()
    expect(mocks.continueHandoffInCodex).toHaveBeenCalledWith('site-1a2b3c4d')
    expect(target.querySelector('[role=alert]')?.textContent).toBe('The terminal could not be opened.')
  })

  it('keeps the Codex button off when Codex is not found', async () => {
    const target = await render(info({ codex_found: false }))
    expect(button(target, 'Continue in Codex: site').disabled).toBe(true)
  })

  it('says so when no note was written yet', async () => {
    const target = await render(info({ notes: [] }))
    expect(target.querySelector('.empty-line')?.textContent?.trim()).toBe('No handoff notes yet')
  })
})
