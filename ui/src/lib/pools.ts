/**
 * How full a pool's reading counts when the widget shows the fullest one, as the tray does: a
 * current reading (also one with use since it) above an old one, and a pool its other window
 * locked as full.
 */
export function poolRank(x: { state: string; used_pct: number | null; blocked: boolean }): number {
  const current = x.state === 'fresh' || x.state === 'behind'
  return current ? 1000 + (x.blocked ? 100 : (x.used_pct ?? -1)) : (x.used_pct ?? -1)
}
