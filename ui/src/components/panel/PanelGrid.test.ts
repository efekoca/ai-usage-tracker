// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import type { LiveCache, PanelData, PanelModule, Settings } from '../../lib/api'
import PanelGrid from './PanelGrid.svelte'

let done: (() => void)[] = []
afterEach(() => {
  done.forEach((d) => d())
  done = []
})

// the tile itself counts down by the real clock
const now = Date.now()
const cache = (i: number) => ({ session_id: `s${i}`, project: `p${i}`, branch: null, model: 'claude-opus-5-5', client: 'cli', ttl_ms: 3_600_000, expires_ms: now + (i + 1) * 600_000, started_ms: now - 3_600_000, tokens: 1000, refresh_usd: 0.1, rewrite_usd: 1, refreshable: true, refreshing: false, refresh_failed: false, unasked: false, auto: false, auto_capped: false, refreshes: 0, refreshes_usd: 0 }) as LiveCache
const tile = { kind: 'caches', provider: '', pool: '', window: '', period: '', action: '', size: 't', style: '', account: '' } as PanelModule
const settings = { cache_timer: true, limit_display: 'used', panel: { value: 'app' } } as unknown as Settings

function render(caches: number, timer = true) {
  const target = document.body.appendChild(document.createElement('div'))
  const data = { caches: Array.from({ length: caches }, (_, i) => cache(i)), cache_refresh: true } as unknown as PanelData
  const c = mount(PanelGrid, { target, props: { modules: [tile], data, settings: { ...settings, cache_timer: timer }, now } })
  flushSync()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  const cell = target.querySelector<HTMLElement>('.cell')!
  return { span: cell.style.getPropertyValue('grid-row'), tall: !!target.querySelector('.mod.caches.t'), rows: target.querySelectorAll('.crow').length }
}

describe('the tall Live caches tile', () => {
  it('is as tall as its conversations, up to three', () => {
    expect(render(0)).toEqual({ span: 'span 1', tall: false, rows: 0 })
    expect(render(1)).toEqual({ span: 'span 1', tall: false, rows: 1 })
    expect(render(2)).toEqual({ span: 'span 2', tall: true, rows: 2 })
    expect(render(3)).toEqual({ span: 'span 3', tall: true, rows: 3 })
    expect(render(5)).toEqual({ span: 'span 3', tall: true, rows: 3 })
  })

  it('takes one row while cache time left is off', () => {
    expect(render(4, false)).toEqual({ span: 'span 1', tall: false, rows: 0 })
  })
})
