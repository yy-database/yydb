import { execSync } from 'node:child_process';
import { existsSync, unlinkSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { RUST_WS_DIR } from './rust-manifest.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const crateDir = join(root, RUST_WS_DIR, 'yydb-wasm');
const outDir = join(root, 'projects/yydb.ts/platforms/yydb-unknown-wasm32/lib');

execSync(`wasm-pack build "${crateDir}" --target web --out-dir "${outDir}" --release`, {
    cwd: root,
    stdio: 'inherit',
    shell: true,
});

// wasm-pack writes `lib/.gitignore` containing `*`, which would keep the
// generated glue and wasm asset out of the packed tarball. The file is local
// build state only, so drop it after the artifacts land.
const libIgnore = join(outDir, '.gitignore');
if (existsSync(libIgnore)) {
    unlinkSync(libIgnore);
}
