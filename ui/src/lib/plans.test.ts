import { describe, expect, it } from 'vitest'
import type { Account, PlansFile, PlanValue, Settings } from './api'
import { accountPlan, autoPlan, planMultiple, planPrice } from './plans'

const plans = {
  providers: {
    anthropic: {
      label: 'Claude',
      plans: [
        { id: 'pro', name: 'Pro', windows: [], source: '', monthly_usd: 20 },
        { id: 'max5x', name: 'Max 5x', windows: [], source: '', monthly_usd: 100 },
        { id: 'max20x', name: 'Max 20x', windows: [], source: '', monthly_usd: 200 },
      ],
    },
    openai: {
      label: 'Codex',
      plans: [
        { id: 'plus', name: 'Plus', windows: [], source: '', monthly_usd: 20 },
        { id: 'business', name: 'Business / Team', windows: [], source: '', monthly_usd: 25 },
      ],
    },
  },
} as unknown as PlansFile

const settings = (over: Partial<Settings> = {}) => ({ plans: {}, plan_prices: {}, account_plans: {}, account_prices: {}, ...over }) as Settings
const account = (read_plan: string | null, id = 'a1') => ({ provider: 'anthropic', id, read_plan }) as Account

describe('accountPlan', () => {
  it('takes the account’s choice, then its readings, then the provider’s plan', () => {
    expect(accountPlan(plans, settings({ account_plans: { a1: 'max20x' }, plans: { anthropic: 'pro' } }), account('pro'))?.id).toBe('max20x')
    expect(accountPlan(plans, settings({ plans: { anthropic: 'max5x' } }), account('pro'))?.id).toBe('pro')
    expect(accountPlan(plans, settings({ plans: { anthropic: 'max20x' } }), account('max'))?.id).toBe('max20x')
    expect(accountPlan(plans, settings({ plans: { anthropic: 'pro' } }), account(null))?.id).toBe('pro')
  })

  it('leaves Max 5x or 20x, and a plan the list lacks, to the user', () => {
    expect(accountPlan(plans, settings({ plans: { anthropic: 'pro' } }), account('max'))).toBeUndefined()
    expect(accountPlan(plans, settings(), account('max'))).toBeUndefined()
    expect(accountPlan(plans, settings({ plans: { anthropic: 'pro' } }), account('unknown'))).toBeUndefined()
  })

  it('reads Claude "team" as either Team plan', () => {
    const team = { ...plans, providers: { ...plans.providers, anthropic: { label: 'Claude', plans: [...plans.providers.anthropic.plans, { id: 'team_standard', name: 'Team (Standard)', windows: [], source: '', monthly_usd: 25 }, { id: 'team_premium', name: 'Team (Premium)', windows: [], source: '', monthly_usd: 125 }] } } } as PlansFile
    expect(accountPlan(team, settings({ plans: { anthropic: 'team_premium' } }), account('team'))?.id).toBe('team_premium')
    expect(accountPlan(team, settings({ plans: { anthropic: 'pro' } }), account('team'))).toBeUndefined()
  })

  it('reads Codex "team" as Business', () => {
    const codex = { provider: 'openai', id: 'c1', read_plan: 'team' } as Account
    expect(accountPlan(plans, settings({ plans: { openai: 'plus' } }), codex)?.id).toBe('business')
  })

  it('names the automatic plan without the account’s own choice', () => {
    const s = settings({ account_plans: { a1: 'max20x' } })
    expect(accountPlan(plans, s, account('pro'))?.id).toBe('max20x')
    expect(autoPlan(plans, s, account('pro'))?.id).toBe('pro')
  })
})

describe('planPrice', () => {
  const pro = plans.providers.anthropic.plans[0]
  const max = plans.providers.anthropic.plans[1]
  it('prefers the account’s own price, then the provider’s on the provider’s plan, then the list price', () => {
    const s = settings({ plans: { anthropic: 'pro' }, plan_prices: { anthropic: 17 }, account_prices: { 'a2:max5x': 90 } })
    expect(planPrice(s, 'anthropic', max, 'a2')).toEqual({ usd: 90, custom: true })
    expect(planPrice(s, 'anthropic', pro, 'a2'), 'a price set on Max stays with Max').toEqual({ usd: 17, custom: true })
    expect(planPrice(s, 'anthropic', pro, 'a1')).toEqual({ usd: 17, custom: true })
    expect(planPrice(s, 'anthropic', max, 'a1')).toEqual({ usd: 100, custom: false })
    expect(planPrice(s, 'anthropic', pro)).toEqual({ usd: 17, custom: true })
    expect(planPrice(s, 'anthropic', undefined, 'a1')).toEqual({ usd: null, custom: false })
    const free = { id: 'free', name: 'Free', windows: [], source: '', monthly_usd: 0 }
    const left = settings({ plans: { anthropic: 'free' }, plan_prices: { anthropic: 24 } })
    expect(planPrice(left, 'anthropic', free), 'a price left from another plan').toEqual({ usd: 0, custom: false })
  })
})

describe('planMultiple', () => {
  const dates = Array.from({ length: 30 }, (_, i) => `2026-10-${String(i + 1).padStart(2, '0')}`)
  const range = { from_ms: new Date(2026, 9, 1).getTime(), to_ms: new Date(2026, 9, 31).getTime() }
  const value = (cost: number, accounts: PlanValue['providers'][number]['accounts'] = []): PlanValue => ({
    range,
    dates,
    providers: [{ provider: 'anthropic', cost_usd: cost, events: 10, unpriced_events: 0, cumulative: [], accounts }],
  })

  it('leaves out a provider without a plan, even with a price kept from an earlier one', () => {
    const v = { plans, settings: settings({ plan_prices: { anthropic: 150 } }), accounts: [], value: value(300) }
    expect(planMultiple(v, ['anthropic'])).toBeNull()
    const pro = { ...v, settings: settings({ plans: { anthropic: 'pro' } }) }
    expect(planMultiple(pro, ['anthropic'])).toBeCloseTo(300 / ((20 * 30) / (365.25 / 12)))
  })

  it('counts a later account only from the day it was first seen', () => {
    const seen = (id: string, day: number) => ({ provider: 'anthropic', id, read_plan: null, first_ms: new Date(2026, 9, day).getTime(), last_ms: range.to_ms - 1, requests: 1, active: true }) as Account
    const use = (account: string, cost: number) => ({ account, cost_usd: cost, events: 5, unpriced_events: 0, cumulative: [] })
    const v = { plans, settings: settings({ account_plans: { a1: 'pro', a2: 'max5x' } }), accounts: [seen('a1', 1), seen('a2', 20)], value: value(160, [use('a1', 60), use('a2', 100)]) }
    const month = 365.25 / 12
    expect(planMultiple({ ...v, fromFirstSeen: true }, ['anthropic'])).toBeCloseTo(160 / ((20 * 30) / month + (100 * 11) / month))
    expect(planMultiple(v, ['anthropic'])).toBeCloseTo(160 / ((20 * 30) / month + (100 * 30) / month))
  })
})
