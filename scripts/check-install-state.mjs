import { execFileSync, execSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const tsDir = join(root, 'projects', 'yydb.ts');

/** Map the host runtime to the optional dependency package that ships its native addon. */
function nativePlatformPackage() {
    const key = `${process.platform}-${process.arch}`;
    const dir = {
        'win32-x64': 'yydb-win32-x64',
        'linux-x64': 'yydb-linux-x64',
        'darwin-x64': 'yydb-darwin-x64',
        'darwin-arm64': 'yydb-darwin-arm64',
    }[key];
    if (!dir) {
        throw new Error(`no YYDB platform package for ${key}`);
    }
    return dir;
}

/** Collect every entry point declared by an `exports` map. */
function exportTargets(value, found = []) {
    if (typeof value === 'string') {
        found.push(value);
    } else if (value && typeof value === 'object') {
        for (const child of Object.values(value)) {
            exportTargets(child, found);
        }
    }
    return found;
}

const targets = [
    {
        name: '@yydb/yydb',
        dir: join(tsDir, 'packages', 'yydb'),
        companions: [join(tsDir, 'platforms', nativePlatformPackage())],
        smoke: [
            "import { loadNative } from '@yydb/yydb';",
            'const version = loadNative().version();',
            "if (typeof version !== 'string' || version.length === 0) throw new Error('native version is empty');",
            "console.log('native version:', version);",
        ].join('\n'),
    },
    {
        name: '@yydb/yydb-client',
        dir: join(tsDir, 'packages', 'yydb-client'),
        companions: [],
        smoke: [
            "import { decodeFrame, encodeFrame } from '@yydb/yydb-client';",
            'const frame = { msgType: 1, flags: 0, requestId: 7, body: new Uint8Array([1, 2, 3]) };',
            'const decoded = decodeFrame(encodeFrame(frame));',
            "if (decoded.requestId !== 7 || decoded.body.length !== 3) throw new Error('wire frame roundtrip failed');",
            "console.log('wire roundtrip requestId:', decoded.requestId);",
        ].join('\n'),
    },
    {
        name: '@yydb/light-db',
        dir: join(tsDir, 'packages', 'light-db'),
        companions: [join(tsDir, 'platforms', 'yydb-unknown-wasm32')],
        smoke: [
            "import { LightDbError, LightDbErrorCodes } from '@yydb/light-db';",
            "const error = new LightDbError(LightDbErrorCodes.corrupt, 'install-state smoke');",
            "if (error.code !== LightDbErrorCodes.corrupt) throw new Error('error code mismatch');",
            "console.log('light-db error code:', error.code);",
        ].join('\n'),
    },
];

const only = process.argv.slice(2).find((arg) => arg.startsWith('--only='));
const selected = targets.filter((target) => !only || target.name === only.slice('--only='.length));
if (selected.length === 0) {
    throw new Error(`no install-state target matched ${only ?? ''}`);
}

function pack(dir, destination, label) {
    execSync(`pnpm pack --pack-destination "${destination}"`, { cwd: dir, stdio: 'inherit' });
    console.log(`packed ${label}`);
}

for (const target of selected) {
    const work = mkdtempSync(join(tmpdir(), 'yydb-install-'));
    const app = join(work, 'app');
    mkdirSync(app, { recursive: true });

    pack(target.dir, work, target.name);
    for (const companion of target.companions) {
        pack(companion, work, companion);
    }

    execSync('npm init -y', { cwd: app, stdio: 'ignore' });
    const archives = readdirSync(work)
        .filter((name) => name.endsWith('.tgz'))
        .map((name) => `"${join(work, name)}"`);
    if (archives.length === 0) {
        throw new Error(`no tarball produced for ${target.name}`);
    }
    execSync(`npm install --no-audit --no-fund ${archives.join(' ')}`, { cwd: app, stdio: 'inherit' });

    const packageDir = join(app, 'node_modules', ...target.name.split('/'));
    const manifest = JSON.parse(readFileSync(join(packageDir, 'package.json'), 'utf8'));
    const entries = exportTargets(manifest.exports);
    if (entries.length === 0) {
        throw new Error(`${target.name} declares no exports`);
    }
    for (const entry of entries) {
        if (entry.endsWith('.d.ts')) {
            continue;
        }
        if (entry.endsWith('.ts')) {
            throw new Error(`${target.name} exports points to TypeScript source: ${entry}`);
        }
        if (!entry.startsWith('./dist/')) {
            throw new Error(`${target.name} exports is outside dist: ${entry}`);
        }
    }
    if (!Array.isArray(manifest.files) || !manifest.files.includes('dist')) {
        throw new Error(`${target.name} files does not ship dist`);
    }
    console.log(`${target.name} exports: ${entries.join(', ')}`);

    execFileSync(process.execPath, ['--input-type=module', '-e', target.smoke], {
        cwd: app,
        stdio: 'inherit',
    });
    console.log(`${target.name} install-state smoke passed`);
}
