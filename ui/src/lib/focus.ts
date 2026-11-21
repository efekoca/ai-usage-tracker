import { tick } from 'svelte'

/**
 * Runs `answer`, then puts the keyboard back on the switch the question was asked from (`from`,
 * else the card's own) once the question is gone: the button it pressed went with it.
 */
export async function backToSwitch(e: Event, answer: () => unknown, gone: () => boolean, from?: HTMLElement | null) {
  const card = (e.currentTarget as HTMLElement | null)?.closest('section')
  await answer()
  await tick()
  if (!gone()) return
  const back = from?.isConnected && card?.contains(from) ? from : card?.querySelector<HTMLElement>('button[role=switch]')
  back?.focus()
}
