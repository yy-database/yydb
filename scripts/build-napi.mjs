import { execSync } from 'node:child_process';
import { copyFileSync, mkdirSync, readdirSync, unlinkSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { NATIVE_ARTIFACTS } from './native-artifacts.mjs';
import { RUST_MANIFEST, RUST_WS_DIR } from './rust-manifest.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const artifact = NATIVE_ARTIFACTS.find((entry) => entry.hostPlatform === process.platform && entry.hostArch === process.arch);

if (!artifact) {
    throw new Error(`No YYDB native package mapping for ${process.platform}-${process.arch}`);
}

execSync(`cargo build --manifest-path "${RUST_MANIFEST}" -p yydb-napi --release`, { cwd: root, stdio: 'inherit', shell: true });

const src = join(root, RUST_WS_DIR, 'target', 'release', artifact.cargoRelease);
const libDir = join(root, 'projects', 'yydb.ts', 'platforms', artifact.pkg, 'lib');
mkdirSync(libDir, { recursive: true });

for (const existing of readdirSync(libDir).filter((name) => name.endsWith('.node'))) {
    unlinkSync(join(libDir, existing));
}

const dest = join(libDir, artifact.fileName);
copyFileSync(src, dest);
console.log(`copied ${src} -> ${dest}`);
