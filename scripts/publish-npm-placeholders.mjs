/**
 * One-time bootstrap: publish empty @yydb/*@0.0.0 packages so Trusted Publisher
 * can be configured on npmjs.com before the first real Release npm run.
 *
 * Usage (logged into npm as a @yydb org publisher):
 *   node scripts/publish-npm-placeholders.mjs
 *   node scripts/publish-npm-placeholders.mjs --dry-run
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const dryRun = process.argv.includes("--dry-run");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const repo = "yy-database/yydb.rs";

const packages = [
    {
        name: "@yydb/yydb-win32-x64",
        directory: "projects/packages/yydb-win32-x64",
        description: "Placeholder — prebuilt yydb engine for win32-x64",
    },
    {
        name: "@yydb/yydb-linux-x64",
        directory: "projects/packages/yydb-linux-x64",
        description: "Placeholder — prebuilt yydb engine for linux-x64",
    },
    {
        name: "@yydb/yydb-darwin-x64",
        directory: "projects/packages/yydb-darwin-x64",
        description: "Placeholder — prebuilt yydb engine for darwin-x64",
    },
    {
        name: "@yydb/yydb-darwin-arm64",
        directory: "projects/packages/yydb-darwin-arm64",
        description: "Placeholder — prebuilt yydb engine for darwin-arm64",
    },
    {
        name: "@yydb/yydb-client",
        directory: "projects/packages/yydb-client",
        description: "Placeholder — YY wire TypeScript client",
    },
    {
        name: "@yydb/yydb",
        directory: "projects/packages/yydb",
        description: "Placeholder — YYDB Node product API",
    },
];

const staging = fs.mkdtempSync(path.join(os.tmpdir(), "yydb-npm-ph-"));
console.log(`staging: ${staging}`);

for (const pkg of packages) {
    const dir = path.join(staging, pkg.name.replace("/", "__"));
    fs.mkdirSync(dir, { recursive: true });
    const packageJson = {
        name: pkg.name,
        version: "0.0.0",
        description: pkg.description,
        license: "MIT",
        private: false,
        publishConfig: { access: "public" },
        repository: {
            type: "git",
            url: `git+https://github.com/${repo}.git`,
            directory: pkg.directory,
        },
        homepage: `https://github.com/${repo}/tree/dev/${pkg.directory}#readme`,
        bugs: { url: `https://github.com/${repo}/issues` },
    };
    fs.writeFileSync(path.join(dir, "package.json"), `${JSON.stringify(packageJson, null, 2)}\n`);
    fs.writeFileSync(
        path.join(dir, "README.md"),
        `# ${pkg.name}\n\nPlaceholder 0.0.0 for npm Trusted Publisher bootstrap.\nReal releases replace this from CI (\`release-npm.yml\`).\n`,
    );

    console.log(
        dryRun ? `[dry-run] would publish ${pkg.name}@0.0.0` : `publishing ${pkg.name}@0.0.0`,
    );
    if (dryRun) continue;

    const r = spawnSync("npm", ["publish", "--access", "public"], {
        cwd: dir,
        stdio: "inherit",
        shell: true,
        env: process.env,
    });
    if (r.status !== 0) {
        console.error(`failed: ${pkg.name}`);
        process.exit(r.status ?? 1);
    }
}

console.log(
    dryRun
        ? "dry-run ok — re-run without --dry-run after npm login"
        : "placeholders published — configure Trusted Publisher on each package",
);
console.log(`workspace root (unchanged): ${root}`);
