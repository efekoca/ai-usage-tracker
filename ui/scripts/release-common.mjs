// Shared by release.mjs and sign-release.mjs so both write the same update addresses.
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

// The project's Tauri CLI, started by node itself: no shell, so paths reach it unchanged.
export function tauri(repo, args, env) {
  return spawnSync(process.execPath, [join(repo, 'ui', 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), ...args], { cwd: repo, env, stdio: 'inherit' })
}

// Where the release files are downloaded from: next to a fixed `endpoint`, or the GitHub release
// of this version. null when config/updates.json names neither, so no latest.json is written.
export function releaseBase(repo, version) {
  const updates = JSON.parse(readFileSync(join(repo, 'config', 'updates.json'), 'utf8'))
  if (updates.endpoint?.startsWith('https://')) return { base: updates.endpoint.slice(0, updates.endpoint.lastIndexOf('/')), updates }
  if (/^[\w.-]+\/[\w.-]+$/.test(updates.github_repo ?? '')) return { base: `https://github.com/${updates.github_repo}/releases/download/v${version}`, updates }
  return { base: null, updates }
}
