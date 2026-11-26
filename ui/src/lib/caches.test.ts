import { describe, expect, it } from 'vitest'
import { clip } from './caches'

describe('clip', () => {
  it('keeps a name that fits and cuts a long one at a whole character', () => {
    expect(clip('ai-usage-tracker', 16)).toBe('ai-usage-tracker')
    expect(clip('customer-portal-backend-services', 16)).toBe('customer-portal…')
    expect(clip('customer-portal-backend', 10)).toBe('customer…')
    expect(clip('ğüşıöçĞÜŞİÖÇğüş', 8)).toBe('ğüşıöçĞ…')
    expect(clip('🧪🧪🧪🧪', 3)).toBe('🧪🧪…')
  })
})
