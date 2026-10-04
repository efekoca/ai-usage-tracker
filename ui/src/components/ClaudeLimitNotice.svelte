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

  const claudeSources = ['claude_code', 'cowork', 'claude_desktop']
  const show = $derived(
    !!st &&
      st.claude_poll &&
      st.claude_candidates_found &&
      st.claude.last_error === 'claude_no_plan_limits' &&
      app.settings?.plans.anthropic !== 'api' &&
      (app.settings?.enabled_sources ?? []).some((s) => claudeSources.includes(s)) &&
      !app.limits.some((l) => l.provider === 'anthropic' && l.source !== 'user_threshold'),
  )
</script>

{#if show}
  <div class="banner" role="status">
    <Icon name="warning" size={16} />
    <span>{t('limits.claudeSignIn')}</span>
    <span class="spacer"></span>
    <button class="btn small" onclick={() => (app.view = 'sources')}>{t('limits.claudeSignIn.action')}</button>
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
