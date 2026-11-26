import { describe, expect, it } from 'vitest'
import type { ModelPrice } from './api'
import { editedModels, ownPrice } from './pricing'

const fields = (over = {}) => ({ model: 'my-model', input: '3', output: '15', cache_read: '', cache_write_5m: '', ...over })

describe('ownPrice', () => {
  it('needs the input and output prices, and prices an empty cache field as input', () => {
    expect(ownPrice(fields({ output: '' }))).toEqual({ error: 'need' })
    expect(ownPrice(fields({ cache_read: '0,3' }))).toMatchObject({ id: 'my-model', input: 3, output: 15, cache_read: 0.3, cache_write_5m: null })
  })

  it('says so when a cache field holds something that is not a price', () => {
    expect(ownPrice(fields({ cache_read: 'abc' }))).toEqual({ error: 'cache' })
    expect(ownPrice(fields({ cache_write_5m: '-1' }))).toEqual({ error: 'cache' })
  })
})

describe('editedModels', () => {
  const price = (id: string, input: number): ModelPrice => ({ id, provider: 'openai', aliases: [], input, output: 1, cache_read: null, cache_write_5m: null, cache_write_1h: null, verified_at: null, notes: null })
  it('keeps only what differs from the list the form was opened on', () => {
    const seen = [price('a', 1), price('b', 2)]
    const form = [price('a', 1), price('b', 3), price('mine', 4)]
    expect(editedModels(form, seen).map((m) => m.id)).toEqual(['b', 'mine'])
  })
})

