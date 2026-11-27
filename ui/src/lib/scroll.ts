// scrollIntoView would also scroll the frames around the page and shift the whole window; parts
// that load later can still move the target or make room below it, so it is followed briefly
// unless the user scrolls
export function scrollPageTo(find: () => HTMLElement | null | undefined) {
  const page = document.querySelector<HTMLElement>('main.content > .page')
  follow(page, () => {
    const el = find()
    return el && page ? Math.max(0, el.getBoundingClientRect().top - page.getBoundingClientRect().top + page.scrollTop - 12) : null
  })
}

/** Back to where the page was left; a page that loads its data after opening grows into it. */
export function scrollPageBack(top: number) {
  follow(document.querySelector<HTMLElement>('main.content > .page'), () => top, false)
}

function follow(page: HTMLElement | null, offset: () => number | null, smooth = true) {
  const start = offset()
  if (!page || start === null) return
  let target = start
  page.scrollTo({ top: target, behavior: smooth && !matchMedia('(prefers-reduced-motion: reduce)').matches ? 'smooth' : 'auto' })
  let height = page.scrollHeight
  const timer = setInterval(() => {
    const now = offset()
    if (now === null) return
    const roomMade = page.scrollHeight !== height && page.scrollTop < now - 1
    height = page.scrollHeight
    if (Math.abs(now - target) > 1 || roomMade) {
      target = now
      page.scrollTo({ top: now })
    }
  }, 100)
  const stop = () => {
    clearInterval(timer)
    for (const e of ['wheel', 'pointerdown', 'keydown']) window.removeEventListener(e, stop, true)
  }
  for (const e of ['wheel', 'pointerdown', 'keydown']) window.addEventListener(e, stop, { capture: true, once: true })
  setTimeout(stop, 3000)
}

/**
 * A click on a table row that should open or close it: not on a control in the row and not the
 * end of a text selection. The second click of a double-click undoes the first, so double-clicking
 * a word only selects it.
 */
export const rowToggles = (e: MouseEvent) =>
  !(e.target as Element | null)?.closest('button, a, input, select, textarea, label') &&
  (e.detail === 2 || (e.detail <= 1 && !getSelection()?.toString()))
