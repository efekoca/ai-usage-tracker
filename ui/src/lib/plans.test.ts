import { describe, expect, it } from 'vitest'
import type { Account, PlansFile, Settings } from './api'
import { accountPlan, autoPlan, planPrice } from './plans'

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
    expect(accountPlan(plans, settings({ plans: { anthropic: 'pro' } }), account('team'))).toBeUndefined()
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
  })
})
