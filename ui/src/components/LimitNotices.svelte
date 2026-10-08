<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { api, type CaptureStatus } from '../lib/api'
  import { t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'

  let st = $state<CaptureStatus | null>(null)

  $effect(() => {
    void app.tick
    api.captureStatus().then((s) => (st = s)).catch(() => {})
  })

  const sources = $derived(app.settings?.enabled_sources ?? [])
  const claudeSources = ['claude_code', 'cowork', 'claude_desktop']
  const claude = $derived(
    !!st &&
      st.claude_poll &&
      st.claude_candidates_found &&
      st.claude.last_error === 'claude_no_plan_limits' &&
      app.settings?.plans.anthropic !== 'api' &&
      sources.some((s) => claudeSources.includes(s)) &&
      !app.limits.some((l) => l.provider === 'anthropic' && l.source !== 'user_threshold'),
  )
  // Antigravity is in use (app or IDE) but only the agy CLI can read its limits
  const agy = $derived(
    !!st &&
      st.antigravity_poll &&
      !st.antigravity_candidates_found &&
      sources.includes('antigravity') &&
      (app.report?.by_tool ?? []).some((g) => g.key === 'antigravity') &&
      !app.limits.some((l) => l.provider === 'google' && l.source !== 'user_threshold'),
  )
</script>

{#if claude}
  <div class="banner" role="status">
    <Icon name="warning" size={16} />
    <span>{t('limits.claudeSignIn')}</span>
    <span class="spacer"></span>
    <button class="btn small" onclick={() => (app.view = 'sources')}>{t('limits.claudeSignIn.action')}</button>
  </div>
{/if}
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
  .spacer {
    flex: 1;
  }
</style>
