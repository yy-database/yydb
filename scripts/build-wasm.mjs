import { execSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { RUST_WS_DIR } from './rust-manifest.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const crateDir = join(root, RUST_WS_DIR, 'ffis/yydb-wasm');
const outDir = join(root, 'projects/yydb.ts/platforms/yydb-unknown-wasm32/lib');

execSync(`wasm-pack build "${crateDir}" --target web --out-dir "${outDir}" --release`, {
    cwd: root,
    stdio: 'inherit',
    shell: true,
});
