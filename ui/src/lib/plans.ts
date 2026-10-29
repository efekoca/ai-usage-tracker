import type { Account, PlanDef, PlansFile, Provider, Settings } from './api'

/** The name a limit reading gives a plan: Claude reports "max" for both Max plans. */
export function readName(p: Provider, id: string): string {
  return p === 'anthropic' && id.startsWith('max') ? 'max' : id
}

/** One of several accounts' plan, as `PlansFile::account_plan` picks it: the one chosen for it, else the one its newest reading names, else the provider's. */
export function accountPlan(plans: PlansFile | null, s: Settings | null, a: Account): PlanDef | undefined {
  const list = plans?.providers[a.provider]?.plans ?? []
  const find = (id: string | undefined) => (id ? list.find((x) => x.id === id) : undefined)
  const own = a.id ? find(s?.account_plans?.[a.id]) : undefined
  if (own) return own
  const provider = find(s?.plans[a.provider])
  if (!a.read_plan) return provider
  const fits = list.filter((x) => readName(a.provider, x.id) === a.read_plan)
  if (!fits.length) return provider
  if (provider && fits.includes(provider)) return provider
  return fits.length === 1 ? fits[0] : undefined
}

/** Monthly price: the account's own, else the provider's own while on the provider's plan, else the list price. */
export function planPrice(s: Settings | null, p: Provider, plan: PlanDef | undefined, account: string | null = null): { usd: number | null; custom: boolean } {
  const mine = account ? s?.account_prices?.[account] : undefined
  if (typeof mine === 'number' && mine > 0) return { usd: mine, custom: true }
  const shared = s?.plan_prices?.[p]
  if ((!account || (plan && plan.id === s?.plans[p])) && typeof shared === 'number' && shared > 0) return { usd: shared, custom: true }
  const list = plan?.monthly_usd
  return { usd: typeof list === 'number' ? list : null, custom: false }
}
