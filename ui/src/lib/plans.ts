import type { Account, PlanDef, PlansFile, Provider, Settings } from './api'

/** Whether a limit reading's plan name fits a plan, as `plans::fits` decides: Claude reports "max" for both Max plans and "team" for both Team plans, Codex "team" for Business and "edu" for Enterprise / Edu. */
export function fits(p: Provider, id: string, read: string): boolean {
  if (p === 'anthropic' && id.startsWith('max')) return read === 'max'
  if (p === 'anthropic' && id.startsWith('team')) return read === 'team'
  if (p === 'openai' && id === 'business') return read === 'business' || read === 'team'
  if (p === 'openai' && id === 'enterprise') return read === 'enterprise' || read === 'edu'
  return id === read
}

/** One of several accounts' plan, as `PlansFile::account_plan` picks it: the one chosen for it, else the one its newest reading names, else the provider's. */
export function accountPlan(plans: PlansFile | null, s: Settings | null, a: Account): PlanDef | undefined {
  const own = a.id ? plans?.providers[a.provider]?.plans.find((x) => x.id === s?.account_plans?.[a.id as string]) : undefined
  return own ?? autoPlan(plans, s, a)
}

/** The plan an account takes when none is chosen for it. */
export function autoPlan(plans: PlansFile | null, s: Settings | null, a: Account): PlanDef | undefined {
  const list = plans?.providers[a.provider]?.plans ?? []
  const provider = list.find((x) => x.id === s?.plans[a.provider])
  if (!a.read_plan) return provider
  const read = a.read_plan
  const fit = list.filter((x) => fits(a.provider, x.id, read))
  if (provider && fit.includes(provider)) return provider
  return fit.length === 1 ? fit[0] : undefined
}

/** Where an account's own price is kept: one per plan, so a price never carries over to another plan. */
export const priceKey = (account: string, plan: string) => `${account}:${plan}`

/** Monthly price: the account's own on this plan, else the provider's own while on the provider's plan, else the list price. A free or API plan never takes a price entered earlier for another plan. */
export function planPrice(s: Settings | null, p: Provider, plan: PlanDef | undefined, account: string | null = null): { usd: number | null; custom: boolean } {
  if (plan?.id === 'api') return { usd: null, custom: false }
  if (plan?.monthly_usd === 0) return { usd: 0, custom: false }
  const mine = account && plan ? s?.account_prices?.[priceKey(account, plan.id)] : undefined
  if (typeof mine === 'number' && mine > 0) return { usd: mine, custom: true }
  const shared = s?.plan_prices?.[p]
  if ((!account || (plan && plan.id === s?.plans[p])) && typeof shared === 'number' && shared > 0) return { usd: shared, custom: true }
  const list = plan?.monthly_usd
  return { usd: typeof list === 'number' ? list : null, custom: false }
}
