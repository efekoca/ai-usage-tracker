import { describe, expect, it } from 'vitest'
import { ownPrice } from './pricing'

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
