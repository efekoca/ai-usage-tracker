// Rust embeds source paths (panic locations) that would carry the builder's user name, so they
// are remapped to neutral prefixes computed here rather than stored in the repository.
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { gzipSync } from 'node:zlib'
import { join, resolve } from 'node:path'
import { releaseBase, tauri } from './release-common.mjs'

const repo = resolve(import.meta.dirname, '..', '..')
const cargoHome = process.env.CARGO_HOME ?? join(homedir(), '.cargo')
const rustupHome = process.env.RUSTUP_HOME ?? join(homedir(), '.rustup')
const remaps = [
  [join(cargoHome, 'registry', 'src'), 'crates'],
  [join(cargoHome, 'git', 'checkouts'), 'git'],
  [rustupHome, 'rustup'],
  [repo, 'app'],
]
// one argument per flag, so paths with spaces stay whole
const sep = '\x1f'
const inherited = process.env.CARGO_ENCODED_RUSTFLAGS?.split(sep) ?? process.env.RUSTFLAGS?.split(/\s+/) ?? []
const flags = [...inherited.filter(Boolean), ...remaps.map(([from, to]) => `--remap-path-prefix=${from}=${to}`)]
const env = { ...process.env, CARGO_ENCODED_RUSTFLAGS: flags.join(sep) }
delete env.RUSTFLAGS

const mac = process.platform === 'darwin'
// arguments after `--` go to cargo
const argv = process.argv.slice(2)
const split = argv.includes('--') ? argv.indexOf('--') : argv.length
const cliArgs = argv.slice(0, split)
// the lock file is part of the release: a stale one stops the build instead of being rewritten
const cargoArgs = [...new Set([...argv.slice(split + 1), '--locked'])]
const targetAt = cliArgs.indexOf('--target')
// one package for Apple silicon and Intel Macs
const target = targetAt >= 0 ? cliArgs[targetAt + 1] : mac ? 'universal-apple-darwin' : null
if (mac && targetAt < 0) cliArgs.push('--target', target)
const notarize = ['APPLE_API_KEY', 'APPLE_API_ISSUER', 'APPLE_API_KEY_PATH'].filter((k) => !process.env[k])
const notarized = !!process.env.APPLE_SIGNING_IDENTITY && notarize.length === 0
if (mac) {
  // Tauri signs with this Keychain identity and notarizes with the App Store Connect API key
  if (!process.env.APPLE_SIGNING_IDENTITY) {
    console.log('APPLE_SIGNING_IDENTITY is not set: the app will not be signed, and macOS will block it on first open.')
  } else if (notarize.length) {
    console.log(`${notarize.join(', ')} not set: the app will be signed but not notarized, and macOS will warn on first open.`)
  }
}

const keyPath = process.env.TAURI_SIGNING_PRIVATE_KEY_PATH ?? join(homedir(), '.tauri', 'ai-usage-tracker.key')
const hasKey = !!process.env.TAURI_SIGNING_PRIVATE_KEY || existsSync(keyPath)
// the updater skips Gatekeeper, so a Mac update must be signed and notarized before it is offered
const signing = hasKey && (!mac || notarized)
if (hasKey && !signing) console.log('Without Apple signing and notarization the Mac build gets no update artifacts.')
const conf = JSON.parse(readFileSync(join(repo, 'src-tauri', 'tauri.conf.json'), 'utf8'))
const version = conf.version
const args = ['build', ...cliArgs]
const scratch = mkdtempSync(join(tmpdir(), 'ai-usage-tracker-'))
const extra = { bundle: {} }
if (signing) {
  env.TAURI_SIGNING_PRIVATE_KEY ??= keyPath // the CLI takes a path or the key itself
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= ''
  extra.bundle.createUpdaterArtifacts = true
} else {
  console.log('No update signing key found: building without update artifacts.')
}
if (process.platform === 'linux') {
  // Tauri files the changelog under the product name; Debian looks for it under the package name
  const changelog = join(scratch, 'changelog.gz')
  writeFileSync(changelog, gzipSync(debianChangelog(conf.mainBinaryName, version), { level: 9 }))
  extra.bundle.linux = { deb: { files: { [`/usr/share/doc/${conf.mainBinaryName}/changelog.gz`]: changelog } } }
}
const extraFile = join(scratch, 'release.json')
writeFileSync(extraFile, JSON.stringify(extra))
args.push('--config', extraFile, '--', ...cargoArgs)

// the Tauri CLI finds src-tauri from the repository root
const r = tauri(repo, args, env)
rmSync(scratch, { recursive: true, force: true })
if (r.status !== 0) process.exit(r.status ?? 1)

// cargo resolves a relative CARGO_TARGET_DIR from the repository root, where the CLI runs
const targetDir = resolve(repo, process.env.CARGO_TARGET_DIR || 'target')
const bundle = join(targetDir, ...(target ? [target] : []), 'release', 'bundle')
const out = join(targetDir, 'release', 'bundle', 'release')
// collected aside first, so a failed run leaves the previous release files where they are
const staging = `${out}.partial`
rmSync(staging, { recursive: true, force: true })
mkdirSync(staging, { recursive: true })
const files = []
const platforms = {}
// names without spaces: release pages rewrite spaces, which would break the update URL
const take = (dir, name, asset) => {
  copyFileSync(join(dir, name), join(staging, asset))
  files.push(asset)
}
const fail = (msg) => {
  console.error(msg)
  rmSync(staging, { recursive: true, force: true })
  process.exit(1)
}

if (mac) {
  const arch = target === 'universal-apple-darwin' ? 'universal' : target.startsWith('aarch64') ? 'aarch64' : 'x64'
  const dmgDir = join(bundle, 'dmg')
  const dmg = existsSync(dmgDir) && readdirSync(dmgDir).find((f) => f.endsWith(`_${version}_${arch}.dmg`))
  if (!dmg) fail(`No disk image for v${version} in ${dmgDir}`)
  // Tauri notarizes the app but not the disk image, which Gatekeeper also checks when a download is opened
  if (notarized) {
    const image = join(dmgDir, dmg)
    const run = (cmd, a) => {
      const r = spawnSync(cmd, a, { stdio: 'inherit' })
      if (r.status !== 0) fail(`${cmd} ${a[0]} failed for ${image}`)
    }
    run('codesign', ['--force', '--sign', process.env.APPLE_SIGNING_IDENTITY, '--timestamp', image])
    run('xcrun', ['notarytool', 'submit', image, '--key', process.env.APPLE_API_KEY_PATH, '--key-id', process.env.APPLE_API_KEY, '--issuer', process.env.APPLE_API_ISSUER, '--wait'])
    run('xcrun', ['stapler', 'staple', image])
  }
  take(dmgDir, dmg, `AI-Usage-Tracker_${version}_${arch}.dmg`)
  if (signing) {
    const app = join(bundle, 'macos')
    const check = (cmd, a) => {
      const r = spawnSync(cmd, a, { stdio: 'inherit' })
      if (r.status !== 0) fail(`${cmd} ${a.join(' ')} failed: not publishing a Mac update`)
    }
    const bundled = join(app, `${conf.productName}.app`)
    check('codesign', ['--verify', '--deep', '--strict', bundled])
    check('spctl', ['--assess', '--type', 'execute', bundled])
    check('xcrun', ['stapler', 'validate', bundled])
    check('xcrun', ['stapler', 'validate', join(dmgDir, dmg)])
    const archive = `${conf.productName}.app.tar.gz`
    if (!existsSync(join(app, archive))) fail(`No update archive in ${app}`)
    const asset = `AI-Usage-Tracker_${version}_${arch}.app.tar.gz`
    take(app, archive, asset)
    take(app, `${archive}.sig`, `${asset}.sig`)
    const keys = arch === 'universal' ? ['darwin-aarch64', 'darwin-x86_64'] : [arch === 'aarch64' ? 'darwin-aarch64' : 'darwin-x86_64']
    for (const k of keys) platforms[k] = { signature: readFileSync(join(app, `${archive}.sig`), 'utf8').trim(), asset }
  }
  // the disk image and the update archive carry the app; the loose copy would show up in Launchpad
  rmSync(join(bundle, 'macos', `${conf.productName}.app`), { recursive: true, force: true })
} else if (process.platform === 'linux') {
  // Tauri names the arch differently per format; assets and manifest keys use the updater's names
  const arch = { x64: 'x86_64', arm64: 'aarch64' }[process.arch] ?? process.arch
  for (const [dir, ext, kind] of [['deb', '.deb', 'deb'], ['rpm', '.rpm', 'rpm'], ['appimage', '.AppImage', 'appimage']]) {
    const from = join(bundle, dir)
    const file = existsSync(from) && readdirSync(from).find((f) => f.endsWith(ext) && f.includes(version))
    if (!file) fail(`No ${ext} package for v${version} in ${from}`)
    const asset = `AI-Usage-Tracker_${version}_${arch}${ext}`
    take(from, file, asset)
    if (signing) {
      take(from, `${file}.sig`, `${asset}.sig`)
      platforms[`linux-${arch}-${kind}`] = { signature: readFileSync(join(from, `${file}.sig`), 'utf8').trim(), asset }
    }
  }
  // an updater that does not know its package type falls back to the AppImage
  if (signing) platforms[`linux-${arch}`] = platforms[`linux-${arch}-appimage`]
} else {
  const nsis = join(bundle, 'nsis')
  const setup = readdirSync(nsis).find((f) => f.endsWith(`_${version}_x64-setup.exe`))
  if (!setup) fail(`No installer for v${version} in ${nsis}`)
  const asset = `AI-Usage-Tracker_${version}_x64-setup.exe`
  take(nsis, setup, asset)
  if (signing) {
    take(nsis, `${setup}.sig`, `${asset}.sig`)
    platforms['windows-x86_64'] = { signature: readFileSync(join(nsis, `${setup}.sig`), 'utf8').trim(), asset }
  }
}

const { base, updates } = releaseBase(repo, version)
if (signing && base) {
  // the other platform's latest.json for this version, so one manifest serves both
  const other = process.env.RELEASE_MANIFEST ? JSON.parse(readFileSync(process.env.RELEASE_MANIFEST, 'utf8')) : null
  if (other && other.version !== version) fail(`RELEASE_MANIFEST is for v${other.version}, not v${version}`)
  const manifest = {
    version,
    notes: process.env.RELEASE_NOTES ?? other?.notes ?? `AI Usage Tracker v${version}`,
    pub_date: other?.pub_date ?? new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
    platforms: { ...other?.platforms },
  }
  for (const [k, p] of Object.entries(platforms)) manifest.platforms[k] = { signature: p.signature, url: `${base}/${p.asset}` }
  writeFileSync(join(staging, 'latest.json'), JSON.stringify(manifest, null, 2) + '\n')
  files.push('latest.json')
}

rmSync(out, { recursive: true, force: true })
renameSync(staging, out)
console.log(`\nRelease files for v${version} in ${out}:`)
for (const f of files) console.log(`  ${f}`)
if (signing && base) {
  console.log(updates.endpoint?.startsWith('https://')
    ? `\nUpload all of them next to ${updates.endpoint}.`
    : `\nCreate the release "v${version}" (tag v${version}) on github.com/${updates.github_repo} and attach all of them.`)
} else if (signing) {
  console.log('\nconfig/updates.json names no release location, so no latest.json was written and the app will not look for updates.')
}

// one entry for this version, pointing at its release notes
function debianChangelog(pkg, version) {
  const maintainer = readFileSync(join(repo, 'src-tauri', 'Cargo.toml'), 'utf8').match(/^authors = \["([^"]+)"/m)?.[1] ?? conf.bundle.publisher
  const date = new Date().toUTCString().replace('GMT', '+0000')
  const repoName = releaseBase(repo, version).updates.github_repo
  return `${pkg} (${version}) stable; urgency=medium\n\n  * Release notes:\n    https://github.com/${repoName}/releases/tag/v${version}\n\n -- ${maintainer}  ${date}\n`
}
