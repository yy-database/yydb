#!/usr/bin/env node
/**
 * Format the YYDB workspace:
 *   - Rust: `cargo fmt`
 *   - JSON / JS / TS: Biome (`indentWidth: 4`)
 *
 *   node scripts/format.mjs           # write
 *   node scripts/format.mjs --check   # check only
 */
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const checkOnly = process.argv.includes("--check");

/**
 * @param {string} file
 * @param {string[]} args
 */
function runFile(file, args) {
    console.log(`$ ${file} ${args.join(" ")}`);
    execFileSync(file, args, {
        cwd: rootDir,
        stdio: "inherit",
        env: process.env,
        shell: process.platform === "win32",
    });
}

console.log(checkOnly ? "=== Format check ===\n" : "=== Format (write) ===\n");

console.log("--- Rust ---");
runFile("cargo", ["fmt", "--all", ...(checkOnly ? ["--check"] : [])]);

console.log("\n--- Biome (json/js/ts, 4 spaces) ---");
if (checkOnly) {
    runFile("pnpm", ["exec", "biome", "format", "--check", "."]);
} else {
    runFile("pnpm", ["exec", "biome", "format", "--write", "."]);
}

console.log(checkOnly ? "\n=== Format check passed ===" : "\n=== Format complete ===");
