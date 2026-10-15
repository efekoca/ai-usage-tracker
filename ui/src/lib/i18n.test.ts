import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import { dicts, fmtInput, i18n, parseNumber, t } from './i18n.svelte'

const langs = ['tr', 'en'] as const
afterEach(() => {
  i18n.lang = 'en'
})

describe('parseNumber', () => {
  it.each(langs)('reads both notations and rejects malformed amounts (%s)', (lang) => {
    i18n.lang = lang
    const cases: [string, number | null][] = [
      ['2', 2],
      ['-3', -3],
      ['0.5', 0.5],
      ['0,5', 0.5],
      ['.5', 0.5],
      ['12,5', 12.5],
      ['1.250,50', 1250.5],
      ['1,250.50', 1250.5],
      ['5.000.000', 5_000_000],
      ['5,000,000', 5_000_000],
      ['1.234.567,89', 1_234_567.89],
      ['$ 1,234.5', 1234.5],
      ['0.075', 0.075],
      ['0,075', 0.075],
      ['', null],
      ['-', null],
      ['...', null],
      [',,', null],
      ['1.2.3', null],
      ['1,2,3', null],
      ['1.25.50', null],
      ['12,5.25', null],
      ['1.2,5', null],
      ['abc', null],
    ]
    for (const [raw, want] of cases) expect(parseNumber(raw), raw).toBe(want)
  })

  it('groups thousands with a single separator in the language of the app', () => {
    i18n.lang = 'tr'
    expect(parseNumber('1.250')).toBe(1250)
    expect(parseNumber('1,250')).toBe(1.25)
    i18n.lang = 'en'
    expect(parseNumber('1,250')).toBe(1250)
    expect(parseNumber('1.250')).toBe(1.25)
  })

  it.each(langs)('reads back what a price field shows (%s)', (lang) => {
    i18n.lang = lang
    for (const v of [0.075, 0.926, 1.875, 6.25, 12.5, 1250, 3]) expect(parseNumber(fmtInput(v))).toBe(v)
  })
})

describe('t', () => {
  it('picks the singular form only for one', () => {
    expect(t('days.streakValue', { n: 1 })).toBe('1 day')
    expect(t('days.streakValue', { n: 2 })).toBe('2 days')
    expect(t('rep.valueLine', { c: '$1', s: '$2', n: 1 })).toContain('for this day')
    expect(t('rep.valueLine', { c: '$1', s: '$2', n: 3 })).toContain('for these 3 days')
  })
})

describe('dictionaries', () => {
  const params = (s: string) => new Set([...s.matchAll(/\{(\w+)[|}]/g)].map((m) => m[1]))

  it('have the same keys in both languages', () => {
    expect(Object.keys(dicts.tr).filter((k) => !(k in dicts.en))).toEqual([])
    expect(Object.keys(dicts.en).filter((k) => !(k in dicts.tr))).toEqual([])
  })

  it('use the same placeholders in both languages', () => {
    const differ = Object.keys(dicts.en).filter((k) => k in dicts.tr && [...params(dicts.en[k])].sort().join() !== [...params(dicts.tr[k])].sort().join())
    expect(differ).toEqual([])
  })

  it('give English counts a singular form', () => {
    // averages and rates are never exactly one
    const fractional = new Set(['forecast.rate', 'subcache.reqPerRun'])
    const missing = Object.entries(dicts.en)
      .filter(([k, s]) => !fractional.has(k) && /\{n\} [a-z]+s\b/.test(s.replace(/\{\w+\|[^{}]*(?:\{\w+\}[^{}]*)*\}/g, '')))
      .map(([k]) => k)
    expect(missing).toEqual([])
  })

  it('hold every key the code asks for', () => {
    const files = (dir: string): string[] =>
      readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? files(join(dir, e.name)) : /\.(svelte|ts)$/.test(e.name) && !/\.test\.ts$|^devmock\.ts$/.test(e.name) ? [join(dir, e.name)] : []))
    const used = new Set<string>()
    for (const f of files(join(import.meta.dirname, '..'))) {
      for (const m of readFileSync(f, 'utf8').matchAll(/\b(?:t|has)\(\s*(['"])([\w.-]+)\1/g)) used.add(m[2])
    }
    expect(used.size).toBeGreaterThan(300)
    expect([...used].filter((k) => !(k in dicts.tr) || !(k in dicts.en))).toEqual([])
  })
})
