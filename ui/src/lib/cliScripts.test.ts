import { execFileSync } from 'node:child_process'
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { cliScripts } from './cliScripts'

const MAC = "'/Applications/AI Usage Tracker.app/Contents/MacOS/ai-usage-tracker' --json"
const hasJq = (() => {
  try {
    execFileSync('jq', ['--version'])
    return true
  } catch {
    return false
  }
})()

describe('cliScripts', () => {
  it('offers the tools each platform has', () => {
    const kinds = (p: string) => cliScripts(MAC, p).map((x) => x.kind)
    expect(kinds('macos')).toEqual(['command', 'tmux', 'sketchybar', 'raycast'])
    expect(kinds('linux')).toEqual(['command', 'tmux'])
    expect(kinds('windows')).toEqual(['command', 'powershell'])
  })

  it('puts the installation’s own command into every script', () => {
    for (const tab of cliScripts(MAC, 'macos')) {
      if (tab.kind === 'command') expect(tab.files[0].code).toBe(MAC)
      const scripts = tab.files.filter((f) => f.code.startsWith('#!'))
      for (const f of scripts) expect(f.code).toContain(`${MAC} | jq -r`)
    }
    const win = "& 'C:/Users/you/AppData/Local/AI Usage Tracker/ai-usage-tracker.exe' --json | Out-String"
    const ps = cliScripts(win, 'windows').find((x) => x.kind === 'powershell')!
    expect(ps.files[0].code.split('\n')[0]).toBe(`$usage = ${win} | ConvertFrom-Json`)
  })

  it.skipIf(process.platform === 'win32' || !hasJq)('prints one status line from the JSON output', () => {
    const dir = mkdtempSync(join(tmpdir(), 'cli-scripts-'))
    try {
      const json = {
        limits: [
          { provider: 'anthropic', window: 'five_hour', used_pct: 46.7 },
          { provider: 'anthropic', window: 'seven_day', used_pct: 64.2 },
          { provider: 'anthropic', window: 'seven_day_opus', used_pct: 10 },
          { provider: 'openai', window: 'five_hour', used_pct: 12 },
          { provider: 'anthropic', window: 'seven_day', used_pct: null },
        ],
        usage: { cost_usd: 12.849 },
      }
      const fake = join(dir, 'fake tracker')
      writeFileSync(fake, `#!/bin/sh\ncat <<'EOF'\n${JSON.stringify(json)}\nEOF\n`)
      chmodSync(fake, 0o755)
      const tabs = cliScripts(`'${fake}' --json`, 'macos')
      for (const kind of ['tmux', 'raycast'] as const) {
        const f = tabs.find((x) => x.kind === kind)!.files[0]
        expect(execFileSync('sh', ['-c', f.code], { encoding: 'utf8' }).trim()).toBe('5h 46% · 7d 64% · $12.84')
      }
      const plugin = tabs.find((x) => x.kind === 'sketchybar')!.files[1].code
      const out = execFileSync('sh', ['-c', plugin.replace('sketchybar --set', 'echo')], { encoding: 'utf8', env: { ...process.env, NAME: 'claude' } })
      expect(out.trim()).toBe('claude label=5h 46% · 7d 64% · $12.84')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
