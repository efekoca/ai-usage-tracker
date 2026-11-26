/** About how wide an axis label of the charts' 11px type is. */
export const labelWidth = (text: string) => text.length * 7

/**
 * Which of `n` evenly spaced points along `iw` pixels get a label `wide` pixels across: the first
 * set flush left, the rest centred on their point, and the last one flush right when it fits
 * beside the one before it.
 */
export function axisPicks(n: number, iw: number, wide: number, least = 90): number[] {
  if (n <= 0) return []
  // a centred label needs half its width clear of the flush-left first one, and the gap between
  const slot = Math.max(least, wide * 1.5 + 8)
  const step = Math.max(1, Math.ceil(n / Math.max(2, Math.floor(iw / slot))))
  const out: number[] = []
  for (let i = 0; i < n; i += step) out.push(i)
  const last = out[out.length - 1]
  const gap = n > 1 ? ((n - 1 - last) * iw) / (n - 1) : 0
  if (last !== n - 1 && gap >= wide * 1.5 + 8) out.push(n - 1)
  return out
}
