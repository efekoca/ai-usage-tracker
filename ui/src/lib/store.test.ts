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
  saveSettings: vi.fn(),
}))
vi.mock('./api', () => ({
  api: mocks,
  on: (event: string, cb: (payload: unknown) => void) => {
    handlers.set(event, cb)
    return Promise.resolve(() => {})
  },
}))

const { app, back, goBack, init, navigate, refresh, saveSettings, setFilter, setPeriod } = await import('./store.svelte')

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

  it('does not bring back data a refresh asked for before "Delete all data"', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval'] })
    const settings = { onboarded: true, theme: 'system', accent: 'blue', language: 'en', currency: 'USD', fx_rate: 1 } as unknown as Settings
    handlers.get('settings-changed')!(settings)
    let answer: (r: Report) => void = () => {}
    mocks.report.mockReturnValueOnce(new Promise<Report>((r) => (answer = r)))
    const asked = refresh()
    handlers.get('settings-changed')!({ ...settings, onboarded: false })
    answer({ hourly: [], range: { from_ms: 0, to_ms: 0 } } as unknown as Report)
    await asked
    expect(app.report).toBeNull()
    expect(app.limits).toEqual([])
  })

  it('stays on onboarding when a settings save sent before "Delete all data" answers after it', async () => {
    const settings = { onboarded: true, theme: 'system', accent: 'blue', language: 'en', currency: 'USD', fx_rate: 1 } as unknown as Settings
    handlers.get('settings-changed')!(settings)
    let answer: (s: Settings) => void = () => {}
    mocks.saveSettings.mockReturnValueOnce(new Promise<Settings>((r) => (answer = r)))
    const saved = saveSettings({ theme: 'dark' })
    await vi.waitFor(() => expect(mocks.saveSettings).toHaveBeenCalled())
    handlers.get('settings-changed')!({ ...settings, onboarded: false })
    answer({ ...settings, theme: 'dark' } as Settings)
    await saved
    expect(app.settings?.onboarded).toBe(false)
  })

  it('does not show the last wipe\'s leftover folders on a later setup', () => {
    const settings = { onboarded: false, theme: 'system', accent: 'blue', language: 'en', currency: 'USD', fx_rate: 1 } as unknown as Settings
    handlers.get('settings-changed')!(settings)
    app.wipeLeft = ['/data/handoff']
    handlers.get('settings-changed')!({ ...settings, onboarded: true })
    expect(app.wipeLeft).toEqual([])
  })

  it('refreshes during a steady stream of changes', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'Date'] })
    handlers.get('settings-changed')!({ onboarded: true, theme: 'system', accent: 'blue', language: 'en', currency: 'USD', fx_rate: 1 } as unknown as Settings)
    setPeriod({ kind: 'all' })
    mocks.report.mockClear()
    await vi.advanceTimersByTimeAsync(20_000)
    mocks.report.mockClear()
    for (let i = 0; i < 80; i++) {
      handlers.get('data-changed')!(undefined)
      await vi.advanceTimersByTimeAsync(200)
    }
    expect(mocks.report).toHaveBeenCalled()
  })
})

describe('moving between pages', () => {
  it('keeps the page a link left for the way back, and a sidebar click forgets it', () => {
    app.view = 'overview'
    navigate('settings:pricing', { from: true })
    expect(app.view).toBe('settings')
    expect(back.view).toBe('overview')
    goBack()
    expect(app.view).toBe('overview')
    expect(back.view).toBeNull()
    navigate('limits:plans', { from: true })
    navigate('daily')
    expect(app.view).toBe('daily')
    expect(back.view).toBeNull()
  })

  it('ignores a page it does not know', () => {
    app.view = 'overview'
    navigate('nowhere:pricing', { from: true })
    expect(app.view).toBe('overview')
    expect(back.view).toBeNull()
  })
})
