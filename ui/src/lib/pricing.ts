import type { ModelPrice, Rates } from './api'
import { parseNumber } from './i18n.svelte'

/** A price typed in for a model no list has: input and output are needed, an empty cache field prices the cache as input. */
export interface OwnFields {
  model: string
  input: string
  output: string
  cache_read: string
  cache_write_5m: string
}

const rate = (v: string) => {
  const n = parseNumber(v)
  return n !== null && n >= 0 ? n : null
}

/** The model price the fields make, or which of them is not a price. */
export function ownPrice(f: OwnFields): ModelPrice | { error: 'need' | 'cache' } {
  const [input, output] = [rate(f.input), rate(f.output)]
  if (input === null || output === null) return { error: 'need' }
  const [cache_read, cache_write_5m] = [f.cache_read, f.cache_write_5m].map((v) => (v.trim() === '' ? null : rate(v)))
  if ((cache_read === null && f.cache_read.trim()) || (cache_write_5m === null && f.cache_write_5m.trim())) return { error: 'cache' }
  return { id: f.model, provider: 'custom', aliases: [], input, output, cache_read, cache_write_5m, cache_write_1h: null, verified_at: null, notes: null }
}

const RATES = ['input', 'output', 'cache_read', 'cache_write_5m', 'cache_write_1h'] as const

/**
 * The models to save: those of the user's own and those whose rates differ from the list the form
 * was opened on. A newer list fetched while the form stayed open is then not pinned at old prices.
 */
export function editedModels(models: ModelPrice[], seen: (Rates & { id: string })[]): ModelPrice[] {
  return models.filter((m) => {
    const b = seen.find((x) => x.id.toLowerCase() === m.id.toLowerCase())
    return !b || RATES.some((k) => (m[k] ?? null) !== (b[k] ?? null))
  })
}

