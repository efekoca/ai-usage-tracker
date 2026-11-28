<script lang="ts">
  import { app, navigate } from '../lib/store.svelte'
  import { api, type CaptureStatus } from '../lib/api'
  import { t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import SignInSteps from './SignInSteps.svelte'

  /** How many notices show, so the page can leave out what they already say. */
  let { shown = $bindable(0) }: { shown?: number } = $props()

  let st = $state<CaptureStatus | null>(null)
  let open = $state<string | null>(null)

  $effect(() => {
    void app.tick
    api.captureStatus().then((s) => (st = s)).catch(() => {})
  })

  const sources = $derived(app.settings?.enabled_sources ?? [])
  const live = (provider: string) => app.limits.some((l) => l.provider === provider && l.source !== 'user_threshold')
  // as LIVE_READING_MS: an older reading, even of a week still running, does not hide a sign-out
  const arriving = (provider: string) =>
    app.limits.some((l) => l.provider === provider && l.active && l.source !== 'user_threshold' && l.observed_ms !== null && Date.now() - l.observed_ms < 15 * 60_000)
  // a CLI that is installed and switched on but signed out reads no limits
  const signedOut = $derived(
    !st
      ? []
      : [
          {
            kind: 'claude' as const,
            on:
              st.claude_poll &&
              st.claude_candidates_found &&
              st.claude.last_error === 'claude_no_plan_limits' &&
              app.settings?.plans.anthropic !== 'api' &&
              sources.some((s) => ['claude_code', 'cowork', 'claude_desktop'].includes(s)) &&
              !arriving('anthropic'),
            binary: st.claude.binary,
          },
          {
            kind: 'codex' as const,
            on:
              st.codex_poll &&
              st.codex_candidates_found &&
              st.codex.last_error === 'codex_not_signed_in' &&
              app.settings?.plans.openai !== 'api' &&
              sources.includes('codex') &&
              !arriving('openai'),
            binary: st.codex.binary,
          },
          {
            kind: 'antigravity' as const,
            on:
              st.antigravity_poll &&
              st.antigravity_candidates_found &&
              st.antigravity.last_error === 'agy_not_signed_in' &&
              app.settings?.plans.google !== 'api' &&
              sources.includes('antigravity') &&
              !arriving('google'),
            binary: st.antigravity.binary,
          },
        ].filter((x) => x.on),
  )
  // Antigravity is in use (app or IDE) but only the agy CLI can read its limits
  const agy = $derived(
    !!st &&
      st.antigravity_poll &&
      !st.antigravity_candidates_found &&
      sources.includes('antigravity') &&
      (app.report?.by_tool ?? []).some((g) => g.key === 'antigravity') &&
      !live('google'),
  )
  $effect(() => {
    shown = signedOut.length + (agy ? 1 : 0)
  })
</script>

{#if signedOut.length}
  <div class="banner warn col" role="status">
    {#each signedOut as s (s.kind)}
      <div class="line">
        <span class="icon-text"><Icon name="warning" size={16} /><span>{t(`limits.signIn.${s.kind}`)}</span></span>
        <button class="btn small" aria-expanded={open === s.kind} onclick={() => (open = open === s.kind ? null : s.kind)}>{t('limits.signIn.how')}</button>
      </div>
      {#if open === s.kind}<SignInSteps kind={s.kind} binary={s.binary} />{/if}
    {/each}
  </div>
{/if}
{#if agy}
  <div class="banner" role="status">
    <span class="icon-text"><Icon name="info" size={16} /><span>{t('limits.agyMissing')}</span></span>
    <button class="btn small" onclick={() => navigate('sources:antigravity', { from: true })}>{t('limits.agyMissing.action')}</button>
  </div>
{/if}

<style>
  .banner {
    align-items: center;
    margin-bottom: 16px;
  }
  .banner.col {
    flex-direction: column;
    align-items: stretch;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
</style>
