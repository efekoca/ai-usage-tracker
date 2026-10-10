<script lang="ts">
  let {
    checked = $bindable(false),
    label,
    disabled = false,
    onchange,
  }: { checked?: boolean; label: string; disabled?: boolean; onchange?: (v: boolean) => void } = $props()
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  class:on={checked}
  onclick={() => {
    // with a handler the owner's value decides, so a failed save never leaves the switch flipped
    if (onchange) onchange(!checked)
    else checked = !checked
  }}
>
  <span></span>
</button>

<style>
  button {
    flex: none;
    width: 40px;
    height: 24px;
    border-radius: 12px;
    border: 0;
    padding: 2px;
    background: var(--hairline-strong);
    transition: background var(--dur) var(--ease);
    display: inline-flex;
  }
  button.on {
    background: var(--accent);
  }
  span {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25);
    transition: transform var(--dur) var(--ease);
  }
  button.on span {
    transform: translateX(16px);
  }
  button:disabled {
    opacity: 0.5;
  }
</style>
