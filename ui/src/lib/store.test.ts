// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Account, LimitView, Report, Settings } from './api'

const handlers = vi.hoisted(() => new Map<string, (payload: unknown) => void>())
const mocks = vi.hoisted(() => ({
  appInfo: vi.fn(),
  getSettings: vi.fn(),
  scanStatus: vi.fn(),
  updateStatus: vi.fn(),
  report: vi.fn(),
  limits: vi.fn(),
  accounts: vi.fn(),
}))
vi.mock('./api', () => ({
  api: mocks,
  on: (event: string, cb: (payload: unknown) => void) => {
    handlers.set(event, cb)
    return Promise.resolve(() => {})
  },
}))

const { app, init, setFilter } = await import('./store.svelte')

afterEach(() => {
  vi.useRealTimers()
})

describe('the app state', () => {
  it('forgets the filter and the loaded data when "Delete all data" sends the app back to onboarding', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval'] })
    const settings = { onboarded: true, theme: 'system', accent: 'blue', language: 'en', currency: 'USD', fx_rate: 1 } as unknown as Settings
    mocks.appInfo.mockResolvedValue({ supports_mica: false, accent_color: null })
    mocks.getSettings.mockResolvedValue(settings)
    mocks.scanStatus.mockResolvedValue({ running: false })
    mocks.updateStatus.mockResolvedValue(null)
    mocks.report.mockResolvedValue({ hourly: [], range: { from_ms: 0, to_ms: 0 } } as unknown as Report)
    mocks.limits.mockResolvedValue([{ provider: 'anthropic' }] as unknown as LimitView[])
    mocks.accounts.mockResolvedValue([{ provider: 'openai', id: 'a1' }] as unknown as Account[])
    await init()
    setFilter({ projects: [3] })
    app.openSession = 'claude_code s1'
    await vi.waitFor(() => expect(mocks.report).toHaveBeenCalledTimes(2))
    expect(app.report).not.toBeNull()

    handlers.get('settings-changed')!({ ...settings, onboarded: false })
    expect(app.filter).toEqual({})
    expect(app.report).toBeNull()
    expect(app.limits).toEqual([])
    expect(app.accounts).toEqual([])
    expect(app.openSession).toBeNull()
  })
})
