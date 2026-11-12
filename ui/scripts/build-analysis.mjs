import { existsSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const root = fileURLToPath(new URL('../../', import.meta.url));
const target = process.env.CARGO_TARGET_DIR || path.join(root, 'target');
const out = path.join(root, 'ui/generated/analysis');
const env = { ...process.env };
if (process.env.PROPHET_ANALYSIS_CARGO_HOME) env.CARGO_HOME = process.env.PROPHET_ANALYSIS_CARGO_HOME;
const scoped = '/workspace/.prophet-cloud/wasm-bindgen/bin/wasm-bindgen';
const bindgen = process.env.WASM_BINDGEN || (existsSync(scoped) ? scoped : 'wasm-bindgen');
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit' });
  if (result.error) { console.error(result.error.message); process.exit(1); }
  if (result.status !== 0) process.exit(result.status || 1);
}
run('cargo', ['build', '--manifest-path', 'crates/prophet-analysis/Cargo.toml', '--release', '--features', 'web', '--target', 'wasm32-unknown-unknown', '--target-dir', target, '--locked']);
mkdirSync(out, { recursive: true });
run(bindgen, [path.join(target, 'wasm32-unknown-unknown/release/prophet_analysis.wasm'), '--target', 'web', '--out-dir', out]);
