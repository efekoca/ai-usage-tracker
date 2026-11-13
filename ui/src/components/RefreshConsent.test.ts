// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it } from 'vitest'
import { ensureRefresh, refresh } from '../lib/refresh.svelte'
import RefreshConsent from './RefreshConsent.svelte'

let done: (() => void)[] = []
afterEach(() => {
  done.forEach((d) => d())
  done = []
  refresh.asking = false
})

function setup() {
  const opener = document.body.appendChild(document.createElement('button'))
  const target = document.body.appendChild(document.createElement('div'))
  const c = mount(RefreshConsent, { target })
  done.push(() => {
    unmount(c)
    target.remove()
    opener.remove()
  })
  opener.focus()
  const answer = ensureRefresh()
  flushSync()
  return { opener, target, answer }
}

const key = (k: string, shiftKey = false) => window.dispatchEvent(new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true }))

describe('the refresh consent dialog', () => {
  it('opens on its main button and keeps Tab inside', () => {
    const { target } = setup()
    const [cancel, ok] = target.querySelectorAll<HTMLButtonElement>('.dialog button')
    expect(document.activeElement).toBe(ok)
    key('Tab')
    expect(document.activeElement).toBe(cancel)
    key('Tab', true)
    expect(document.activeElement).toBe(ok)
  })

  it('closes on Escape, answers no and gives the focus back', async () => {
    const { opener, target, answer } = setup()
    key('Escape')
    flushSync()
    expect(await answer).toBe(false)
    expect(target.querySelector('.dialog')).toBeNull()
    expect(document.activeElement).toBe(opener)
  })
})
