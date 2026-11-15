export type ScriptKind = 'command' | 'tmux' | 'sketchybar' | 'raycast' | 'powershell'
export type ScriptFile = { name: string; code: string }
export type ScriptTab = { kind: ScriptKind; label: string; files: ScriptFile[] }

/** One line for a status bar: Claude's five-hour and weekly use, and today's cost. */
const STATUS_LINE = String.raw`jq -r '
  [ .limits[]
    | select(.provider == "anthropic" and .used_pct != null and (.window == "five_hour" or .window == "seven_day"))
    | "\(if .window == "five_hour" then "5h" else "7d" end) \(.used_pct | floor)%" ]
  + [ "$\(.usage.cost_usd * 100 | floor / 100)" ]
  | join(" · ")'`

/** `command` is the installation's own JSON command, so every script carries its real path. */
export function cliScripts(command: string, platform: string): ScriptTab[] {
  if (platform === 'windows') {
    return [
      { kind: 'command', label: 'PowerShell', files: [{ name: 'PowerShell', code: command }] },
      {
        kind: 'powershell',
        label: 'PowerShell',
        files: [
          {
            name: 'PowerShell',
            code: `$usage = ${command} | ConvertFrom-Json\n$usage.limits | Where-Object provider -eq anthropic | Format-Table window, used_pct, resets_at`,
          },
        ],
      },
    ]
  }
  const status = `${command} | ${STATUS_LINE}`
  const tabs: ScriptTab[] = [
    { kind: 'command', label: 'Terminal', files: [{ name: 'Terminal', code: command }] },
    {
      kind: 'tmux',
      label: 'tmux',
      files: [
        { name: '~/bin/claude-status.sh', code: `#!/bin/sh\n${status}` },
        { name: '~/.tmux.conf', code: "set -g status-interval 60\nset -g status-right '#(~/bin/claude-status.sh)'" },
      ],
    },
  ]
  if (platform !== 'macos') return tabs
  return [
    ...tabs,
    {
      kind: 'sketchybar',
      label: 'SketchyBar',
      files: [
        { name: 'sketchybarrc', code: 'sketchybar --add item claude right --set claude update_freq=120 script="$PLUGIN_DIR/claude.sh"' },
        { name: 'plugins/claude.sh', code: `#!/bin/sh\nlabel=$(${status})\nsketchybar --set "$NAME" label="$label"` },
      ],
    },
    {
      kind: 'raycast',
      label: 'Raycast',
      files: [
        {
          name: 'claude-usage.sh',
          code: `#!/bin/sh\n# @raycast.schemaVersion 1\n# @raycast.title Claude usage\n# @raycast.mode inline\n# @raycast.refreshTime 5m\n${status}`,
        },
      ],
    },
  ]
}
