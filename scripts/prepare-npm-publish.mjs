import fs from 'node:fs';
import path from 'node:path';

const root = process.cwd();

/** Prefer explicit INPUT_VERSION; else strip leading `v` from tag `vX.Y.Z`. */
function resolveVersion() {
    const input = (process.env.INPUT_VERSION || '').trim().replace(/^v/i, '');
    if (input) return input;
    const ref = (process.env.GITHUB_REF || '').trim();
    const fromRef = ref.match(/^refs\/tags\/v(.+)$/i);
    if (fromRef) return fromRef[1];
    const name = (process.env.GITHUB_REF_NAME || '').trim();
    if (/^v\d+\.\d+\.\d+/i.test(name)) return name.slice(1);
    return '';
}

const version = resolveVersion();

function walkFiles(dir, acc = []) {
    if (!fs.existsSync(dir)) return acc;
    for (const name of fs.readdirSync(dir)) {
        const full = path.join(dir, name);
        const st = fs.statSync(full);
        if (st.isDirectory()) walkFiles(full, acc);
        else acc.push(full);
    }
    return acc;
}

function findNativeBinding(fileName) {
    const files = walkFiles(path.join(root, '_engines'));
    const hit = files.find((f) => path.basename(f) === fileName);
    if (!hit) {
        throw new Error(`missing native binding ${fileName} under _engines/`);
    }
    return hit;
}

function writeJson(filePath, data) {
    fs.writeFileSync(filePath, `${JSON.stringify(data, null, 2)}\n`);
}

function preparePackageJson(pkgDir, { workspaceClientToVersion = false } = {}) {
    const pkgPath = path.join(pkgDir, 'package.json');
    const j = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
    delete j.private;
    if (version) {
        j.version = version;
        if (j.optionalDependencies) {
            for (const key of Object.keys(j.optionalDependencies)) {
                j.optionalDependencies[key] = version;
            }
        }
    }
    if (workspaceClientToVersion && j.dependencies?.['@yydb/yydb-client'] === 'workspace:*') {
        j.dependencies['@yydb/yydb-client'] = version || j.version;
    }
    if (workspaceClientToVersion && j.dependencies?.['@yydb/yydb-unknown-wasm32'] === 'workspace:*') {
        j.dependencies['@yydb/yydb-unknown-wasm32'] = version || j.version;
    }
    // Provenance: repository.url must match the publishing GitHub repo.
    // Keep monorepo `directory` so npm links to projects/packages/<pkg>.
    const ghRepo = (process.env.GITHUB_REPOSITORY || '').trim() || 'yy-database/yydb';
    const directory = path.relative(root, pkgDir).split(path.sep).join('/');
    j.repository = {
        type: 'git',
        url: `git+https://github.com/${ghRepo}.git`,
        directory,
    };
    j.homepage = `https://github.com/${ghRepo}/tree/dev/${directory}#readme`;
    j.bugs = { url: `https://github.com/${ghRepo}/issues` };
    j.publishConfig = { ...(j.publishConfig || {}), access: 'public' };
    if (!j.license) j.license = 'MPL-2.0';
    writeJson(pkgPath, j);
}

import { NATIVE_ARTIFACTS } from './native-artifacts.mjs';

for (const { pkg, fileName } of NATIVE_ARTIFACTS) {
    const destDir = path.join(root, 'projects', 'packages', pkg);
    const libDir = path.join(destDir, 'lib');
    const dest = path.join(libDir, fileName);
    fs.mkdirSync(libDir, { recursive: true });
    fs.copyFileSync(findNativeBinding(fileName), dest);
    try {
        fs.chmodSync(dest, 0o755);
    } catch {
        // windows
    }
    preparePackageJson(destDir);
}

preparePackageJson(path.join(root, 'projects', 'packages', 'yydb-client'));
preparePackageJson(path.join(root, 'projects', 'packages', 'yydb-unknown-wasm32'));
preparePackageJson(path.join(root, 'projects', 'packages', 'yydb'), {
    workspaceClientToVersion: true,
});

console.log('npm packages prepared', version || '(package.json versions)');
