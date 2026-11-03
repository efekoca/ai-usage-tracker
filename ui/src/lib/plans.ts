import type { Account, AccountValue, PlanDef, PlansFile, PlanValue, Provider, Settings } from './api'

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

const DAYS_PER_MONTH = 365.25 / 12

/** What a plan's value is worked out from. */
export interface ValueInput {
  plans: PlansFile | null
  settings: Settings | null
  accounts: Account[]
  value: PlanValue | null
  /** Count each account's plan only from the day it was first seen, for periods longer than its use. It runs on to the period's end: a plan can be paid while unused, so the value stays a lower bound. */
  fromFirstSeen?: boolean
}

function planOf(v: ValueInput, p: Provider): PlanDef | undefined {
  const id = v.settings?.plans[p]
  return id ? v.plans?.providers[p]?.plans.find((x) => x.id === id) : undefined
}

const shareOf = (v: ValueInput, usd: number | null, days = v.value?.dates.length ?? 30) => (usd !== null ? (usd * days) / DAYS_PER_MONTH : null)
const dayOf = (ms: number) => {
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

/** A provider on one plan: its use against the plan's price for the period's days. */
export function valueRow(v: ValueInput, p: Provider) {
  const pv = v.value?.providers.find((x) => x.provider === p)
  const plan = planOf(v, p)
  const price = planPrice(v.settings, p, plan)
  const cost = pv?.cost_usd ?? 0
  const share = shareOf(v, price.usd)
  const ratio = share && share > 0 ? cost / share : null
  const day = share !== null && pv ? pv.cumulative.findIndex((c) => c >= share) : -1
  return { pv, plan, price, cost, share, ratio, day }
}

/** A provider with several accounts: each account seen in the period on its own plan, against their prices together. */
export function valueSplit(v: ValueInput, p: Provider) {
  const pv = v.value?.providers.find((x) => x.provider === p)
  const range = v.value?.range
  const parts = v.accounts
    .filter((a) => a.provider === p && a.id !== null && a.last_ms >= (range?.from_ms ?? 0) && a.first_ms < (range?.to_ms ?? Infinity))
    .map((a) => {
      const id = a.id as string
      const plan = accountPlan(v.plans, v.settings, a)
      const price = planPrice(v.settings, p, plan, id)
      const days = v.fromFirstSeen ? v.value?.dates.filter((d) => d >= dayOf(a.first_ms)).length : undefined
      return { id, plan, price, share: shareOf(v, price.usd, days), use: pv?.accounts.find((x) => x.account === id) }
    })
  const unknown = pv?.accounts.find((x) => x.account === null)
  // use billed at API rates, or on a free plan, is not paid for by the plans counted here
  const counted = parts.filter((x) => x.plan?.id !== 'api' && x.price.usd !== 0)
  const missing = counted.find((x) => !x.plan || x.share === null)
  const uses = [...counted.map((x) => x.use), unknown].filter((u): u is AccountValue => !!u)
  const cost = uses.reduce((t, u) => t + u.cost_usd, 0)
  const cumulative = (v.value?.dates ?? []).map((_, i) => uses.reduce((t, u) => t + (u.cumulative[i] ?? 0), 0))
  const share = counted.reduce((t, x) => t + (x.share ?? 0), 0)
  return {
    pv,
    parts,
    unknown,
    missing,
    cost,
    cumulative,
    share,
    ratio: share > 0 ? cost / share : null,
    day: share > 0 ? cumulative.findIndex((c) => c >= share) : -1,
    monthly: counted.reduce((t, x) => t + (x.price.usd ?? 0), 0),
    custom: counted.some((x) => x.price.custom),
    accounts: counted.length,
    events: uses.reduce((t, u) => t + u.events, 0),
    unpriced: uses.reduce((t, u) => t + u.unpriced_events, 0),
  }
}

/** API-equivalent use of `providers` over what their paid plans cost for the period. A provider whose plan or price is unknown is left out of both, even with a price kept from an earlier plan; `null` when no used provider is on a known paid plan. */
export function planMultiple(v: ValueInput, providers: Provider[]): number | null {
  let cost = 0
  let share = 0
  for (const p of providers) {
    if ((v.value?.providers.find((x) => x.provider === p)?.events ?? 0) === 0) continue
    const m = v.accounts.some((a) => a.provider === p) ? valueSplit(v, p) : null
    const row = m && m.parts.length ? null : valueRow(v, p)
    const part = m && m.parts.length ? (m.missing ? null : m) : row?.plan ? row : null
    if (part?.share) {
      cost += part.cost
      share += part.share
    }
  }
  return share > 0 ? cost / share : null
}
