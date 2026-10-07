// Signs release files built elsewhere (CI has no update signing key) and adds them to latest.json.
// Usage: npm --prefix ui run sign-release -- <folder>
// Every package without a .sig is signed; every signature must name this version. latest.json in the
// folder (or RELEASE_MANIFEST) is merged, and SHA256SUMS lists every download of this version.
import { createHash } from 'node:crypto'
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'
import { releaseBase, tauri } from './release-common.mjs'

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
  [/_(aarch64|x64)\.app\.tar\.gz$/, (a) => [a === 'x64' ? 'darwin-x86_64' : 'darwin-aarch64']],
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
delete env.TAURI_SIGNING_PRIVATE_KEY_PATH // the CLI refuses a key and a key path together
env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= ''

const platforms = {}
for (const file of readdirSync(dir).filter((f) => f.includes(`_${version}_`))) {
  const kind = kinds.find(([re]) => re.test(file))
  if (!kind) {
    // disk images, signatures and the manifest are expected; anything else may be a missed package
    if (!/\.(dmg|sig|json)$/.test(file)) console.warn(`Skipping ${file}: not a package the updater knows`)
    continue
  }
  if (!existsSync(join(dir, `${file}.sig`))) {
    // installed apps require the version in the signature (`requireSignedVersion`)
    const r = tauri(repo, ['signer', 'sign', '--app-version', version, join(dir, file)], env)
    if (r.status !== 0) process.exit(r.status ?? 1)
  }
  const signature = readFileSync(join(dir, `${file}.sig`), 'utf8').trim()
  const trusted = Buffer.from(signature, 'base64').toString('utf8').split('\n').find((l) => l.startsWith('trusted comment:')) ?? ''
  if (!trusted.split('\t').includes(`version:${version}`)) {
    console.error(`${file}.sig is not bound to v${version}, so installed apps would reject the update. Delete it and run this again.`)
    process.exit(1)
  }
  for (const key of kind[1](file.match(kind[0])[1])) {
    if (platforms[key]) {
      console.error(`${platforms[key].file} and ${file} are both for ${key}; keep one of them`)
      process.exit(1)
    }
    platforms[key] = { signature, file }
  }
}
if (!Object.keys(platforms).length) {
  console.error(`No release files for v${version} in ${dir}`)
  process.exit(1)
}

const { base } = releaseBase(repo, version)
if (!base) {
  console.error('config/updates.json names no release location (an https endpoint or an owner/name github_repo)')
  process.exit(1)
}
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

// for people who download by hand: `sha256sum -c SHA256SUMS --ignore-missing`
const downloads = readdirSync(dir).filter((f) => f.includes(`_${version}_`) && !f.endsWith('.sig')).sort()
const sums = downloads.map((f) => `${createHash('sha256').update(readFileSync(join(dir, f))).digest('hex')}  ${f}\n`).join('')
writeFileSync(join(dir, 'SHA256SUMS'), sums)
console.log(`SHA256SUMS lists ${downloads.length} files`)
