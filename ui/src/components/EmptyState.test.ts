// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('../lib/api', () => ({
  api: { report: vi.fn(() => new Promise(() => {})), limits: vi.fn(() => new Promise(() => {})) },
  on: () => Promise.resolve(() => {}),
}))

const { app } = await import('../lib/store.svelte')
const { default: EmptyState } = await import('./EmptyState.svelte')

let done: (() => void)[] = []
afterEach(() => {
  done.forEach((d) => d())
  done = []
})

function render() {
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(EmptyState, { target, props: {} })
  flushSync()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}
const buttons = (t: HTMLElement) => [...t.querySelectorAll('button')].map((b) => b.textContent)

describe('an empty page', () => {
  it('offers to clear the filters and widen the period only when they narrow the view', () => {
    app.period = { kind: 'all' }
    app.filter = {}
    expect(buttons(render())).toEqual(['Open sources'])

    app.period = { kind: 'days7' }
    app.filter = { models: ['gpt-5.6'] }
    const t = render()
    expect(buttons(t)).toEqual(['Clear filters', 'Show all time', 'Open sources'])
    t.querySelector('button')!.click()
    flushSync()
    expect(app.filter).toEqual({})
    expect(buttons(t)).toEqual(['Show all time', 'Open sources'])
  })
})
