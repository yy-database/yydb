/**
 * Configure npm Trusted Publisher for @yydb/* (GitHub Actions OIDC).
 *
 * Auth: NPM_TOTP_SECRET or NPM_OTP in .env.placeholder.local (same as placeholder publish).
 * Optional: NPM_TOKEN in .env.placeholder.local or ~/.npmrc for registry API fallback.
 *
 *   node scripts/configure-trusted-publishers.mjs
 *   node scripts/configure-trusted-publishers.mjs --dry-run
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createHmac } from 'node:crypto';
import { spawnSync } from 'node:child_process';

const root = process.cwd();
const dryRun = process.argv.includes('--dry-run');

const TRUST_REPO = 'yy-database/yydb';
const TRUST_FILE = 'release-npm.yml';
const TRUST_ENV = 'NPM_PUBLISH';

const PACKAGES = [
    '@yydb/yydb-win32-x64',
    '@yydb/yydb-linux-x64',
    '@yydb/yydb-darwin-x64',
    '@yydb/yydb-darwin-arm64',
    '@yydb/yydb-client',
    '@yydb/yydb',
];

function loadEnvFile(filePath) {
    const out = new Map();
    if (!fs.existsSync(filePath)) return out;
    for (const raw of fs.readFileSync(filePath, 'utf8').split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('#')) continue;
        const eq = line.indexOf('=');
        if (eq < 0) continue;
        const key = line.slice(0, eq).trim().replace(/^export\s+/, '');
        let value = line.slice(eq + 1).trim();
        if (
            (value.startsWith('"') && value.endsWith('"')) ||
            (value.startsWith("'") && value.endsWith("'"))
        ) {
            value = value.slice(1, -1);
        }
        out.set(key, value);
    }
    return out;
}

function envOrFile(key) {
    const fromProcess = process.env[key]?.trim();
    if (fromProcess) return fromProcess;
    const local = loadEnvFile(path.join(root, '.env.placeholder.local'));
    return local.get(key)?.trim() || '';
}

function decodeBase32(secret) {
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
    const cleaned = secret.replace(/[\s=-]/g, '').toUpperCase();
    let bits = '';
    for (const ch of cleaned) {
        const index = alphabet.indexOf(ch);
        if (index < 0) throw new Error('invalid base32 in NPM_TOTP_SECRET');
        bits += index.toString(2).padStart(5, '0');
    }
    const bytes = [];
    for (let i = 0; i + 8 <= bits.length; i += 8) {
        bytes.push(Number.parseInt(bits.slice(i, i + 8), 2));
    }
    if (!bytes.length) throw new Error('NPM_TOTP_SECRET decoded empty');
    return Buffer.from(bytes);
}

function totpCode(secret) {
    const key = decodeBase32(secret);
    const counter = Math.floor(Date.now() / 1000 / 30);
    const buf = Buffer.alloc(8);
    buf.writeBigUInt64BE(BigInt(counter));
    const digest = createHmac('sha1', key).update(buf).digest();
    const offset = digest[digest.length - 1] & 0x0f;
    const code =
        ((digest.readUInt32BE(offset) & 0x7fffffff) % 1_000_000).toString().padStart(6, '0');
    return code;
}

function currentOtp() {
    const staticOtp = envOrFile('NPM_OTP') || envOrFile('OTP');
    if (/^\d{6}$/.test(staticOtp || '')) return staticOtp;
    const secret = envOrFile('NPM_TOTP_SECRET') || envOrFile('TOTP_SECRET');
    if (!secret) {
        throw new Error(
            'Set NPM_TOTP_SECRET or NPM_OTP in .env.placeholder.local (same as placeholder publish)',
        );
    }
    return totpCode(secret);
}

function extractJson(text) {
    const trimmed = String(text || '').trim();
    if (!trimmed) return null;
    const start = trimmed.search(/[\[{]/);
    if (start < 0) return null;
    try {
        return JSON.parse(trimmed.slice(start));
    } catch {
        return null;
    }
}

function npm(args, { otp } = {}) {
    const argv = [...args];
    if (otp) argv.push(`--otp=${otp}`);
    const r = spawnSync('npm', argv, { encoding: 'utf8', shell: true });
    const blob = `${r.stdout || ''}\n${r.stderr || ''}`;
    return { status: r.status ?? 1, stdout: r.stdout || '', stderr: r.stderr || '', blob };
}

function listTrust(packageName, otp) {
    const r = npm(['trust', 'list', packageName, '--json'], { otp });
    if (r.blob.includes('EOTP') || r.blob.includes('one-time password')) {
        throw new Error(`npm trust list needs OTP for ${packageName}`);
    }
    if (r.status !== 0) {
        throw new Error(`npm trust list failed for ${packageName}: ${r.blob.trim()}`);
    }
    const data = extractJson(r.stdout);
    if (!data) return [];
    if (Array.isArray(data)) return data;
    if (Array.isArray(data.configurations)) return data.configurations;
    if (Array.isArray(data.items)) return data.items;
    if (data && typeof data === 'object') return [data];
    return [];
}

function trustMatches(configs) {
    return configs.some((entry) => {
        const repo = entry?.repository || entry?.claims?.repository;
        const file = entry?.file || entry?.claims?.workflow_ref?.file || entry?.claims?.workflowFilename;
        const env = entry?.environment || entry?.claims?.environment;
        return repo === TRUST_REPO && file === TRUST_FILE && (!env || env === TRUST_ENV);
    });
}

function configureTrust(packageName, otp) {
    if (dryRun) {
        console.log(`would configure trusted publisher for ${packageName}`);
        return 'configured';
    }
    const r = npm(
        [
            'trust',
            'github',
            packageName,
            `--file=${TRUST_FILE}`,
            `--repo=${TRUST_REPO}`,
            `--env=${TRUST_ENV}`,
            '--allow-publish',
            '--allow-stage-publish',
            '--yes',
        ],
        { otp },
    );
    if (r.status === 0) {
        console.log(`configured ${packageName}`);
        return 'configured';
    }
    if (r.blob.includes('already exists') || r.blob.includes('409')) {
        console.log(`already configured ${packageName}`);
        return 'skipped';
    }
    throw new Error(`npm trust github failed for ${packageName}: ${r.blob.trim()}`);
}

console.log(`trusted publisher target: ${TRUST_REPO} / ${TRUST_FILE} / ${TRUST_ENV}`);
const otp = currentOtp();

const summary = { configured: [], skipped: [], failed: [] };
for (const packageName of PACKAGES) {
    process.stdout.write(`\n=== ${packageName} ===\n`);
    try {
        const configs = listTrust(packageName, otp);
        if (trustMatches(configs)) {
            console.log('already matches');
            summary.skipped.push(packageName);
            continue;
        }
        const status = configureTrust(packageName, otp);
        summary[status === 'configured' ? 'configured' : 'skipped'].push(packageName);
    } catch (err) {
        console.error(String(err.message || err));
        summary.failed.push(packageName);
    }
}

console.log('\n=== summary ===');
console.log(JSON.stringify(summary, null, 2));
if (summary.failed.length) process.exit(1);
