// Signs release files built elsewhere (CI has no update signing key) and adds them to latest.json.
// Usage: npm --prefix ui run sign-release -- <folder>
// Every package without a .sig is signed; latest.json in the folder (or RELEASE_MANIFEST) is merged.
import { spawnSync } from 'node:child_process'
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'

const repo = resolve(import.meta.dirname, '..', '..')
const dir = process.argv[2] ? resolve(process.cwd(), process.argv[2]) : null
if (!dir || !existsSync(dir)) {
  console.error('Usage: npm --prefix ui run sign-release -- <folder with the release files>')
  process.exit(1)
}
const version = JSON.parse(readFileSync(join(repo, 'src-tauri', 'tauri.conf.json'), 'utf8')).version

// the updater looks up `{os}-{arch}-{bundle type}` first, then `{os}-{arch}`
const kinds = [
  [/_x64-setup\.exe$/, () => ['windows-x86_64']],
  [/_universal\.app\.tar\.gz$/, () => ['darwin-aarch64', 'darwin-x86_64']],
  [/_(aarch64|x86_64)\.deb$/, (a) => [`linux-${a}-deb`]],
  [/_(aarch64|x86_64)\.rpm$/, (a) => [`linux-${a}-rpm`]],
  [/_(aarch64|x86_64)\.AppImage$/, (a) => [`linux-${a}-appimage`, `linux-${a}`]],
]

const env = { ...process.env }
const keyPath = env.TAURI_SIGNING_PRIVATE_KEY_PATH ?? join(homedir(), '.tauri', 'ai-usage-tracker.key')
if (!env.TAURI_SIGNING_PRIVATE_KEY && !existsSync(keyPath)) {
  console.error(`No update signing key: set TAURI_SIGNING_PRIVATE_KEY or put it at ${keyPath}`)
  process.exit(1)
}
env.TAURI_SIGNING_PRIVATE_KEY ??= readFileSync(keyPath, 'utf8')
env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= ''

const platforms = {}
for (const file of readdirSync(dir).filter((f) => f.includes(`_${version}_`))) {
  const kind = kinds.find(([re]) => re.test(file))
  if (!kind) continue
  if (!existsSync(join(dir, `${file}.sig`))) {
    const r = spawnSync('npx', ['--prefix', 'ui', 'tauri', 'signer', 'sign', `"${join(dir, file)}"`], { cwd: repo, env, stdio: 'inherit', shell: true })
    if (r.status !== 0) process.exit(r.status ?? 1)
  }
  const signature = readFileSync(join(dir, `${file}.sig`), 'utf8').trim()
  for (const key of kind[1](file.match(kind[0])[1])) platforms[key] = { signature, file }
}
if (!Object.keys(platforms).length) {
  console.error(`No release files for v${version} in ${dir}`)
  process.exit(1)
}

const updates = JSON.parse(readFileSync(join(repo, 'config', 'updates.json'), 'utf8'))
const base = updates.endpoint?.startsWith('https://')
  ? updates.endpoint.slice(0, updates.endpoint.lastIndexOf('/'))
  : `https://github.com/${updates.github_repo}/releases/download/v${version}`
const existing = process.env.RELEASE_MANIFEST ?? join(dir, 'latest.json')
const other = existsSync(existing) ? JSON.parse(readFileSync(existing, 'utf8')) : null
if (other && other.version !== version) {
  console.error(`${existing} is for v${other.version}, not v${version}`)
  process.exit(1)
}
const manifest = {
  version,
  notes: process.env.RELEASE_NOTES ?? other?.notes ?? `AI Usage Tracker v${version}`,
  pub_date: other?.pub_date ?? new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
  platforms: { ...other?.platforms },
}
for (const [key, p] of Object.entries(platforms)) manifest.platforms[key] = { signature: p.signature, url: `${base}/${p.file}` }
writeFileSync(join(dir, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n')
console.log(`\nlatest.json for v${version} now lists: ${Object.keys(manifest.platforms).join(', ')}`)
