// scrollIntoView would also scroll the frames around the page and shift the whole window; parts
// that load later can still move the target or make room below it, so it is followed briefly
// unless the user scrolls
export function scrollPageTo(find: () => HTMLElement | null | undefined) {
  const page = document.querySelector<HTMLElement>('main.content > .page')
  const offset = () => {
    const el = find()
    return el && page ? el.getBoundingClientRect().top - page.getBoundingClientRect().top + page.scrollTop : null
  }
  const start = offset()
  if (!page || start === null) return
  let target = start
  page.scrollTo({ top: target, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' })
  let height = page.scrollHeight
  const follow = setInterval(() => {
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
    clearInterval(follow)
    for (const e of ['wheel', 'pointerdown', 'keydown']) window.removeEventListener(e, stop, true)
  }
  for (const e of ['wheel', 'pointerdown', 'keydown']) window.addEventListener(e, stop, { capture: true, once: true })
  setTimeout(stop, 1500)
}

/**
 * A click on a table row that should open or close it: not on a control in the row and not the
 * end of a text selection. The second click of a double-click undoes the first, so double-clicking
 * a word only selects it.
 */
export const rowToggles = (e: MouseEvent) =>
  !(e.target as Element | null)?.closest('button, a, input, select, textarea, label') &&
  (e.detail === 2 || (e.detail <= 1 && !getSelection()?.toString()))
