// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { HandoffInfo, Settings } from '../lib/api'

const mocks = vi.hoisted(() => ({
  handoffInfo: vi.fn(),
  setLimitHandoff: vi.fn(),
  setCodexHandoff: vi.fn(),
  openHandoffNote: vi.fn(),
  continueHandoff: vi.fn(),
}))
vi.mock('../lib/api', () => ({ api: mocks, on: () => Promise.resolve(() => {}) }))

const { app } = await import('../lib/store.svelte')
const { default: LimitHandoff } = await import('./LimitHandoff.svelte')

const folder = '/Users/you/Library/Application Support/ai-usage-tracker/handoff/notes'
const drop = '/var/folders/x/T/ai-usage-tracker-handoff-501'
const info = (over: Partial<HandoffInfo> = {}): HandoffInfo => ({
  installed: true,
  settings_file: '/Users/you/.claude/settings.json',
  folder,
  codex_found: true,
  claude_found: true,
  codex_here: true,
  codex_installed: true,
  codex_hooks_file: '/Users/you/.codex/hooks.json',
  codex_seen: true,
  codex_folder: drop,
  notes: [
    { key: 'site-1a2b3c4d', project: 'site', written_ms: Date.now() - 60_000, agent: 'claude' },
    { key: 'api-5e6f7a8b', project: 'api', written_ms: Date.now() - 120_000, agent: 'codex' },
  ],
  ...over,
})

let done: (() => void)[] = []
beforeEach(() => {
  app.settings = { limit_handoff: true, codex_handoff: true } as unknown as Settings
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

const header = (t: HTMLElement) => t.querySelector<HTMLButtonElement>('.head button[role=switch]')!
const button = (t: HTMLElement, label: string) => t.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
const confirm = (t: HTMLElement) => [...t.querySelectorAll<HTMLButtonElement>('.ask .actions button')].at(-1)!

describe('the handoff note', () => {
  it('names both hooks, their files and the folders before adding them', async () => {
    app.settings = { limit_handoff: false, codex_handoff: false } as unknown as Settings
    const target = await render(info({ installed: false, codex_installed: false, notes: [] }))
    header(target).click()
    flushSync()
    const ask = target.querySelector('.ask')!
    expect(ask.querySelector('.claude')?.textContent).toContain('/Users/you/.claude/settings.json')
    expect(ask.querySelector('.claude')?.textContent).toContain(folder)
    expect(ask.querySelector('.codex')?.textContent).toContain('/Users/you/.codex/hooks.json')
    expect(ask.querySelector('.codex')?.textContent).toContain(drop)
    expect(mocks.setLimitHandoff).not.toHaveBeenCalled()
    mocks.setLimitHandoff.mockResolvedValue(info({ codex_installed: false, notes: [] }))
    mocks.setCodexHandoff.mockResolvedValue(info({ notes: [] }))
    confirm(target).click()
    await settle()
    expect(mocks.setLimitHandoff).toHaveBeenCalledWith(true)
    expect(mocks.setCodexHandoff).toHaveBeenCalledWith(true)
    expect(target.querySelector('.ask')).toBeNull()
  })

  it('asks only about Claude when Codex is not on this computer', async () => {
    app.settings = { limit_handoff: false, codex_handoff: false } as unknown as Settings
    const target = await render(info({ installed: false, codex_installed: false, codex_here: false, notes: [] }))
    header(target).click()
    flushSync()
    expect(target.querySelector('.ask .claude')).not.toBeNull()
    expect(target.querySelector('.ask .codex')).toBeNull()
  })

  it('turns Codex on by itself after asking about Codex alone, and says when Codex has not run the hook yet', async () => {
    app.settings = { limit_handoff: true, codex_handoff: false } as unknown as Settings
    const target = await render(info({ codex_installed: false, codex_seen: false }))
    expect(target.querySelector('.tools')?.textContent).toContain('Off')
    button(target, 'Codex').click()
    flushSync()
    expect(target.querySelector('.ask .claude')).toBeNull()
    expect(target.querySelector('.ask .codex')).not.toBeNull()
    mocks.setCodexHandoff.mockResolvedValue(info({ codex_seen: false }))
    confirm(target).click()
    await settle()
    expect(mocks.setLimitHandoff).not.toHaveBeenCalled()
    expect(mocks.setCodexHandoff).toHaveBeenCalledWith(true)
    expect(target.querySelector('.tools')?.textContent).toContain('Trust the hook when Codex asks')
  })

  it('goes on in the other tool than the one that wrote the note', async () => {
    const target = await render(info())
    const rows = [...target.querySelectorAll('.row.note')]
    expect(rows[0].textContent).toContain('Claude ·')
    expect(rows[1].textContent).toContain('Codex ·')
    button(target, 'Open the note: site').click()
    await settle()
    expect(mocks.openHandoffNote).toHaveBeenCalledWith('site-1a2b3c4d')
    button(target, 'Continue in Codex: site').click()
    await settle()
    expect(mocks.continueHandoff).toHaveBeenCalledWith('site-1a2b3c4d')
    mocks.continueHandoff.mockRejectedValue('terminal_failed')
    button(target, 'Continue in Claude: api').click()
    await settle()
    expect(mocks.continueHandoff).toHaveBeenCalledWith('api-5e6f7a8b')
    expect(target.querySelector('[role=alert]')?.textContent).toBe('The terminal could not be opened.')
  })

  it('keeps a button off when its tool is not found', async () => {
    const target = await render(info({ codex_found: false, claude_found: false }))
    expect(button(target, 'Continue in Codex: site').disabled).toBe(true)
    expect(button(target, 'Continue in Claude: api').disabled).toBe(true)
    expect(button(target, 'Continue in Claude: api').title).toBe('Claude Code was not found.')
  })

  it('says so when no note was written yet', async () => {
    const target = await render(info({ notes: [] }))
    expect(target.querySelector('.empty-line')?.textContent?.trim()).toBe('No handoff notes yet')
  })
})
