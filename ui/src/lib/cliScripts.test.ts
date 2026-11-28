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

  it.skipIf(process.platform === 'win32' || !hasJq)('prints one status line from the JSON output, marking a reading that is behind', () => {
    const dir = mkdtempSync(join(tmpdir(), 'cli-scripts-'))
    const fake = join(dir, 'fake tracker')
    const json = join(dir, 'out.json')
    writeFileSync(fake, `#!/bin/sh\ncat '${json}'\n`)
    chmodSync(fake, 0o755)
    const tabs = cliScripts(`'${fake}' --json`, 'macos')
    const limit = (window: string, used_pct: number | null, state = 'fresh', budget = false) => ({ provider: 'anthropic', window, used_pct, state, budget })
    const week = limit('seven_day', 64.2)
    const line = (five: object) => {
      writeFileSync(json, JSON.stringify({ limits: [five, week, limit('seven_day_opus', 10), { ...limit('five_hour', 12), provider: 'openai' }], usage: { cost_usd: 12.849 } }))
      const outs = (['tmux', 'raycast'] as const).map((kind) => execFileSync('sh', ['-c', tabs.find((x) => x.kind === kind)!.files[0].code], { encoding: 'utf8' }).trim())
      const plugin = tabs.find((x) => x.kind === 'sketchybar')!.files[1].code.replace('sketchybar --set', 'echo')
      outs.push(execFileSync('sh', ['-c', plugin], { encoding: 'utf8', env: { ...process.env, NAME: 'claude' } }).trim().replace(/^claude label=/, ''))
      expect(new Set(outs).size).toBe(1)
      return outs[0]
    }
    try {
      expect(line(limit('five_hour', 46.7))).toBe('5h 46% | 7d 64% | $12.84')
      expect(line(limit('five_hour', 46.7, 'behind'))).toBe('5h ~46% | 7d 64% | $12.84')
      expect(line(limit('five_hour', 46.7, 'stale'))).toBe('7d 64% | $12.84')
      expect(line(limit('five_hour', null, 'reset'))).toBe('7d 64% | $12.84')
      expect(line(limit('five_hour', 46.7, 'fresh', true))).toBe('7d 64% | $12.84')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
