// Release build.
// 1. No build-machine paths in the binary: Rust embeds source paths (panic locations), which
//    would carry the builder's user name. They are remapped to neutral prefixes computed here,
//    so no machine-specific path is ever written into the repository.
// 2. Signed updates: when the update signing key exists (TAURI_SIGNING_PRIVATE_KEY, or the file
//    at TAURI_SIGNING_PRIVATE_KEY_PATH, default ~/.tauri/ai-usage-tracker.key) the installer is
//    signed and, when config/updates.json names a release location, a latest.json is written
//    next to it. Both go to target/release/bundle/release, ready to attach to a release.
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const repo = resolve(import.meta.dirname, '..', '..')
const cargoHome = process.env.CARGO_HOME ?? join(homedir(), '.cargo')
const rustupHome = process.env.RUSTUP_HOME ?? join(homedir(), '.rustup')
const remaps = [
  [join(cargoHome, 'registry', 'src'), 'crates'],
  [join(cargoHome, 'git', 'checkouts'), 'git'],
  [rustupHome, 'rustup'],
  [repo, 'app'],
]
const flags = remaps.map(([from, to]) => `--remap-path-prefix=${from}=${to}`).join(' ')
const env = { ...process.env, RUSTFLAGS: [process.env.RUSTFLAGS, flags].filter(Boolean).join(' ') }

const keyPath = process.env.TAURI_SIGNING_PRIVATE_KEY_PATH ?? join(homedir(), '.tauri', 'ai-usage-tracker.key')
const signing = !!process.env.TAURI_SIGNING_PRIVATE_KEY || existsSync(keyPath)
const args = ['--prefix', 'ui', 'tauri', 'build', ...process.argv.slice(2)]
if (signing) {
  env.TAURI_SIGNING_PRIVATE_KEY ??= keyPath // the CLI takes a path or the key itself
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= ''
  const extra = join(tmpdir(), 'ai-usage-tracker-release.json')
  writeFileSync(extra, JSON.stringify({ bundle: { createUpdaterArtifacts: true } }))
  args.push('--config', extra)
} else {
  console.log('No update signing key found: building without update artifacts.')
}

// run from the repository root, where the Tauri CLI finds src-tauri
const r = spawnSync('npx', args, { cwd: repo, env, stdio: 'inherit', shell: true })
if (r.status !== 0) process.exit(r.status ?? 1)

const version = JSON.parse(readFileSync(join(repo, 'src-tauri', 'tauri.conf.json'), 'utf8')).version
const nsis = join(repo, 'target', 'release', 'bundle', 'nsis')
const setup = readdirSync(nsis).find((f) => f.endsWith(`_${version}_x64-setup.exe`))
if (!setup) {
  console.error(`No installer for v${version} in ${nsis}`)
  process.exit(1)
}
const out = join(repo, 'target', 'release', 'bundle', 'release')
rmSync(out, { recursive: true, force: true })
mkdirSync(out, { recursive: true })
// a name without spaces: release pages rewrite spaces, which would break the update URL
const asset = `AI-Usage-Tracker_${version}_x64-setup.exe`
copyFileSync(join(nsis, setup), join(out, asset))
const files = [asset]

const updates = JSON.parse(readFileSync(join(repo, 'config', 'updates.json'), 'utf8'))
const base = updates.endpoint?.startsWith('https://')
  ? updates.endpoint.slice(0, updates.endpoint.lastIndexOf('/'))
  : /^[\w.-]+\/[\w.-]+$/.test(updates.github_repo ?? '')
    ? `https://github.com/${updates.github_repo}/releases/download/v${version}`
    : null
if (signing) {
  const sig = readFileSync(join(nsis, `${setup}.sig`), 'utf8').trim()
  copyFileSync(join(nsis, `${setup}.sig`), join(out, `${asset}.sig`))
  files.push(`${asset}.sig`)
  if (base) {
    const manifest = {
      version,
      notes: process.env.RELEASE_NOTES ?? `AI Usage Tracker v${version}`,
      pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
      platforms: { 'windows-x86_64': { signature: sig, url: `${base}/${asset}` } },
    }
    writeFileSync(join(out, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n')
    files.push('latest.json')
  }
}

console.log(`\nRelease files for v${version} in ${out}:`)
for (const f of files) console.log(`  ${f}`)
if (signing && base) {
  console.log(updates.endpoint?.startsWith('https://')
    ? `\nUpload all of them next to ${updates.endpoint}.`
    : `\nCreate the release "v${version}" (tag v${version}) on github.com/${updates.github_repo} and attach all of them.`)
} else if (signing) {
  console.log('\nconfig/updates.json names no release location, so no latest.json was written and the app will not look for updates.')
}
