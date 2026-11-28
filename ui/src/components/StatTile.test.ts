// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import StatTile from './StatTile.svelte'

let done: (() => void)[] = []
afterEach(() => {
  done.forEach((d) => d())
  done = []
})

function render(props: { current?: number; previous?: number; points?: boolean; hint?: string }) {
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(StatTile, { target, props: { label: 'Rate', value: 'x', ...props } })
  flushSync()
  done.push(() => {
    unmount(c)
    target.remove()
  })
  return target
}

describe('StatTile', () => {
  it('shows a percentage change in points, with its direction', () => {
    const t = render({ current: 66, previous: 60, points: true })
    expect(t.querySelector('.delta')?.textContent).toContain('6.0 points')
    expect(t.querySelector('.delta.up svg')).not.toBeNull()
    expect(t.querySelector('.sr-only')?.textContent).toBe('Up')
  })

  it('gives a change that rounds to zero no arrow and no direction', () => {
    const t = render({ current: 95.6, previous: 95.6, points: true })
    expect(t.querySelector('.delta')?.textContent).toContain('0.0 points')
    expect(t.querySelector('.delta svg')).toBeNull()
    expect(t.querySelector('.delta.up, .delta.down, .sr-only')).toBeNull()
  })

  it('shows other values as a relative change', () => {
    const t = render({ current: 80, previous: 100 })
    expect(t.querySelector('.delta')?.textContent).toContain('20%')
    expect(t.querySelector('.delta.down svg')).not.toBeNull()
    expect(t.querySelector('.sr-only')?.textContent).toBe('Down')
  })

  it('opens its hint from a button and closes it with Escape', () => {
    const t = render({ hint: 'Read tokens over prompt tokens.' })
    const q = t.querySelector<HTMLButtonElement>('.q')!
    expect(q.getAttribute('aria-expanded')).toBe('false')
    expect(t.querySelector('.pop')).toBeNull()
    q.click()
    flushSync()
    expect(q.getAttribute('aria-expanded')).toBe('true')
    expect(t.querySelector('.pop')?.textContent).toBe('Read tokens over prompt tokens.')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    flushSync()
    expect(t.querySelector('.pop')).toBeNull()
  })

  it('shows no button without a hint', () => {
    expect(render({}).querySelector('.q')).toBeNull()
  })
})
