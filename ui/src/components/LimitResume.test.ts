// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ResumeInfo, Settings } from '../lib/api'

const mocks = vi.hoisted(() => ({
  limitResumeInfo: vi.fn(),
  setLimitResume: vi.fn(),
  cancelLimitResume: vi.fn(),
}))
vi.mock('../lib/api', () => ({ api: mocks, on: () => Promise.resolve(() => {}) }))

const { app } = await import('../lib/store.svelte')
const { default: LimitResume } = await import('./LimitResume.svelte')

const now = Date.now()
const file = '/Users/you/.claude/settings.json'
const info = (over: Partial<ResumeInfo> = {}): ResumeInfo => ({ installed: true, settings_file: file, waiting: [], resumed: [], ...over })
const waiting = (over: Partial<ResumeInfo['waiting'][number]> = {}): ResumeInfo['waiting'][number] => ({ session_id: 's', project: 'notes', branch: 'main', client: 'cli', stopped_ms: now - 60_000, window: 'five_hour', resumes_ms: now + 3_600_000, held: false, ...over })

let done: (() => void)[] = []
beforeEach(() => {
  app.settings = { limit_resume: true } as unknown as Settings
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

async function render(first: ResumeInfo) {
  mocks.limitResumeInfo.mockResolvedValue(first)
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(LimitResume, { target })
  await settle()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}

const rows = (t: HTMLElement) => [...t.querySelectorAll('.list-row')].map((r) => r.textContent?.replace(/\s+/g, ' ').trim())
const toggle = (t: HTMLElement) => t.querySelector<HTMLButtonElement>('button[role=switch]')!

describe('continuing after a limit resets', () => {
  it('lists the conversations that wait and those continued', async () => {
    const target = await render(
      info({
        waiting: [waiting(), waiting({ session_id: 'u', project: '', branch: null, window: null, resumes_ms: null }), waiting({ session_id: 'h', held: true }), waiting({ session_id: 'd', resumes_ms: now - 1000 })],
        resumed: [{ session_id: 'r', project: 'site', branch: null, at_ms: now - 60_000 }],
      }),
    )
    const [known, unknown, held, due, went] = rows(target)
    expect(known).toMatch(/^notes · main Terminal · 5-hour limit full continues at \d/)
    expect(unknown).toBe("Hidden project Terminal Reset time not known yet Don't continue")
    expect(held).toContain('Continued too often today')
    expect(due).toContain('Continues shortly')
    expect(went).toMatch(/^site continued at \d/)
    expect(toggle(target).getAttribute('aria-checked')).toBe('true')
  })

  it('calls one conversation off', async () => {
    const target = await render(info({ waiting: [waiting()] }))
    mocks.cancelLimitResume.mockResolvedValue(undefined)
    mocks.limitResumeInfo.mockResolvedValue(info())
    target.querySelector<HTMLButtonElement>('.list-row button')!.click()
    await settle()
    expect(mocks.cancelLimitResume).toHaveBeenCalledWith('s')
    expect(rows(target)).toEqual([])
    expect(target.textContent).toContain('No conversation stopped by a limit is waiting.')
  })

  it('asks before it adds the hook, and says why it could not', async () => {
    app.settings = { limit_resume: false } as unknown as Settings
    const target = await render(info({ installed: false }))
    expect(toggle(target).getAttribute('aria-checked')).toBe('false')
    toggle(target).click()
    flushSync()
    expect(mocks.setLimitResume).not.toHaveBeenCalled()
    expect(target.querySelector('.ask')?.textContent).toContain(file)

    mocks.setLimitResume.mockRejectedValueOnce('settings file is not valid JSON')
    const ok = target.querySelector<HTMLButtonElement>('.ask .primary')!
    ok.click()
    await settle()
    expect(target.querySelector('.ask [role=alert]')?.textContent).toContain('not valid JSON')

    mocks.setLimitResume.mockImplementationOnce(async () => {
      app.settings = { limit_resume: true } as unknown as Settings
      return info()
    })
    ok.click()
    await settle()
    expect(mocks.setLimitResume).toHaveBeenLastCalledWith(true)
    expect(target.querySelector('.ask')).toBeNull()
    expect(toggle(target).getAttribute('aria-checked')).toBe('true')
  })

  it('turns off without asking, and is off while the hook is gone', async () => {
    const target = await render(info({ installed: false, waiting: [waiting()] }))
    expect(toggle(target).getAttribute('aria-checked')).toBe('false')
    expect(rows(target)).toEqual([])
    const on = await render(info({ waiting: [waiting()] }))
    mocks.setLimitResume.mockImplementationOnce(async () => {
      app.settings = { limit_resume: false } as unknown as Settings
      return info({ installed: false })
    })
    toggle(on).click()
    await settle()
    expect(mocks.setLimitResume).toHaveBeenCalledWith(false)
    expect(on.querySelector('.ask')).toBeNull()
    expect(rows(on)).toEqual([])
  })
})
