#!/usr/bin/env node
/**
 * Format the YYDB workspace:
 *   - Rust: `cargo fmt`
 *   - JSON / JS / TS: Nifty (`nifty.config.ts`, 4 spaces)
 *
 *   node scripts/format.mjs           # write
 *   node scripts/format.mjs --check   # check only
 *
 * Known upstream defect: the published `@doki-land/nifty` formatter spaces
 * reserved member names (`delete (key: string)`) and rewrites optional method
 * signatures (`name?(...): T`) into a ternary, so the JS/TS check stays red on
 * `projects/yydb.ts/**` until a nifty release ships the Oak formatter fix.
 * The workspace intentionally keeps no exclusion or ignore rule for this.
 */
import { execFileSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { RUST_MANIFEST } from './rust-manifest.mjs';

const rootDir = join(dirname(fileURLToPath(import.meta.url)), '..');
const checkOnly = process.argv.includes('--check');

/**
 * @param {string} file
 * @param {string[]} args
 */
function runFile(file, args) {
    console.log(`$ ${file} ${args.join(' ')}`);
    execFileSync(file, args, {
        cwd: rootDir,
        stdio: 'inherit',
        env: process.env,
        shell: process.platform === 'win32',
    });
}

console.log(checkOnly ? '=== Format check ===\n' : '=== Format (write) ===\n');

console.log('--- Rust ---');
runFile('cargo', ['fmt', '--manifest-path', RUST_MANIFEST, '--all', ...(checkOnly ? ['--check'] : [])]);

console.log('\n--- Nifty (json/js/ts) ---');
runFile('pnpm', ['exec', 'nifty', 'format', ...(checkOnly ? ['--check'] : [])]);

console.log(checkOnly ? '\n=== Format check passed ===' : '\n=== Format complete ===');
