// Fails on high and critical npm advisories, as `npm audit --audit-level=high` does, except the
// ones below. Each is skipped only while npm knows no fix that keeps every package on its major
// version, so the run fails again once a fixed release is out: then update the lockfile and drop
// the entry. `node scripts/audit.mjs report.json` checks a saved `npm audit --json` report.
import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const WAITING = {}
const SEVERE = new Set(['high', 'critical'])

const ui = join(dirname(fileURLToPath(import.meta.url)), '..')
const text = process.argv[2]
  ? readFileSync(process.argv[2], 'utf8')
  : spawnSync('npm', ['audit', '--json'], { cwd: ui, encoding: 'utf8', maxBuffer: 64 << 20, shell: process.platform === 'win32' }).stdout
let report = null
try {
  report = JSON.parse(text)
} catch {}
if (!report?.vulnerabilities) {
  console.error('npm audit gave no report:', report?.error?.summary ?? String(text).slice(0, 500))
  process.exit(1)
}

const fixable = (fix) => fix === true || (typeof fix === 'object' && fix !== null && !fix.isSemVerMajor)

const failed = new Set()
const skipped = new Set()
const seen = new Set()
for (const v of Object.values(report.vulnerabilities)) {
  for (const a of v.via) {
    if (typeof a !== 'object') continue
    const id = String(a.url ?? '').split('/').pop()
    seen.add(id)
    if (!SEVERE.has(a.severity)) continue
    const line = `${a.name} (${a.severity}): ${a.title} ${a.url}`
    if (!(id in WAITING)) failed.add(line)
    else if (fixable(v.fixAvailable)) failed.add(`${line}\n  a fix is out: update the lockfile and drop ${id} from ui/scripts/audit.mjs`)
    else skipped.add(`skipped ${id} until a fix is out: ${WAITING[id]}`)
  }
}
for (const s of skipped) console.log(s)
for (const id of Object.keys(WAITING).filter((id) => !seen.has(id))) console.log(`${id} is no longer reported; drop it from ui/scripts/audit.mjs`)
if (failed.size) {
  console.error([...failed].join('\n'))
  process.exit(1)
}
console.log('no other high or critical advisories')
