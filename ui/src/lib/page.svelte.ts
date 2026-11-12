/** Whether this window is on screen; its timers and reads wait while it is hidden. */
export const page = $state({ visible: !document.hidden })

document.addEventListener('visibilitychange', () => (page.visible = !document.hidden))
