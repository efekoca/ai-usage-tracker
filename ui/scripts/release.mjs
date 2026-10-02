// Release build without build-machine paths in the binary: Rust embeds source paths (panic
// locations), which would carry the builder's user name. They are remapped to neutral prefixes
// computed here, so no machine-specific path is ever written into the repository.
import { spawnSync } from 'node:child_process'
import { homedir } from 'node:os'
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
// run from the repository root, where the Tauri CLI finds src-tauri
const r = spawnSync('npx', ['--prefix', 'ui', 'tauri', 'build', ...process.argv.slice(2)], { cwd: repo, env, stdio: 'inherit', shell: true })
process.exit(r.status ?? 1)
