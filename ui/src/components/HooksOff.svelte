<script lang="ts">
  import { has, t } from '../lib/i18n.svelte'
  import Icon from './Icon.svelte'

  /** The Claude Code setting that keeps the app's hooks from running, if one does. */
  let { setting }: { setting: string | null | undefined } = $props()
</script>

{#if setting && has(`hooks.off.${setting}`)}
  <p class="hooks-off small" role="note"><Icon name="info" size={13} />{t(`hooks.off.${setting}`)}</p>
{/if}

<style>
  .hooks-off {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    color: var(--bad-ink);
  }
  .hooks-off :global(svg) {
    flex: none;
    margin-top: 2px;
  }
</style>
