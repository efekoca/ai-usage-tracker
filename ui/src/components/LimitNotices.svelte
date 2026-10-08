<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, type CaptureStatus } from '../lib/api'
  import { t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'
  import SignInSteps from './SignInSteps.svelte'

  let st = $state<CaptureStatus | null>(null)
  let open = $state<string | null>(null)

  $effect(() => {
    void app.tick
    api.captureStatus().then((s) => (st = s)).catch(() => {})
  })

  const sources = $derived(app.settings?.enabled_sources ?? [])
  const live = (provider: string) => app.limits.some((l) => l.provider === provider && l.source !== 'user_threshold')
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
              !live('anthropic'),
            binary: st.claude.binary,
          },
          {
            kind: 'codex' as const,
            on: st.codex_poll && st.codex_candidates_found && st.codex.last_error === 'codex_not_signed_in' && sources.includes('codex') && !live('openai'),
            binary: st.codex.binary,
          },
          {
            kind: 'antigravity' as const,
            on:
              st.antigravity_poll &&
              st.antigravity_candidates_found &&
              st.antigravity.last_error === 'agy_not_signed_in' &&
              sources.includes('antigravity') &&
              !live('google'),
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
</script>

{#each signedOut as s (s.kind)}
  <div class="banner col" role="status">
    <div class="line">
      <Icon name="warning" size={16} />
      <span>{t(`limits.signIn.${s.kind}`)}</span>
      <span class="spacer"></span>
      <button class="btn small" aria-expanded={open === s.kind} onclick={() => (open = open === s.kind ? null : s.kind)}>{t('limits.signIn.how')}</button>
    </div>
    {#if open === s.kind}<SignInSteps kind={s.kind} binary={s.binary} />{/if}
  </div>
{/each}
{#if agy}
  <div class="banner" role="status">
    <Icon name="info" size={16} />
    <span>{t('limits.agyMissing')}</span>
    <span class="spacer"></span>
    <button class="btn small" onclick={() => (app.view = 'sources')}>{t('limits.agyMissing.action')}</button>
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
  .spacer {
    flex: 1;
  }
</style>
