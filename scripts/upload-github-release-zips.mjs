/**
 * Create (if needed) a GitHub Release for the current tag and upload engine zips.
 * Existing release / existing assets are skipped (retry-safe).
 *
 * Env:
 *   GITHUB_REF_NAME — tag (e.g. v0.1.0)
 *   GITHUB_REPOSITORY — owner/repo
 *   GITHUB_TOKEN / GH_TOKEN — contents:write
 *   ENGINES_DIR — directory with downloaded engine-* artifacts (default _engines)
 *   ZIPS_DIR — where to write zips (default _release-zips)
 */
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const tag = (process.env.GITHUB_REF_NAME || "").trim();
const repo = (process.env.GITHUB_REPOSITORY || "").trim();
const enginesDir = process.env.ENGINES_DIR || "_engines";
const zipsDir = process.env.ZIPS_DIR || "_release-zips";

if (!tag.startsWith("v")) {
    console.error("GITHUB_REF_NAME must be a v* tag");
    process.exit(1);
}
if (!repo) {
    console.error("GITHUB_REPOSITORY is required");
    process.exit(1);
}

const packages = [
    { pkg: "yydb-win32-x64", bin: "yydb.exe" },
    { pkg: "yydb-linux-x64", bin: "yydb" },
    { pkg: "yydb-darwin-x64", bin: "yydb" },
    { pkg: "yydb-darwin-arm64", bin: "yydb" },
];

function run(cmd, args, opts = {}) {
    return spawnSync(cmd, args, {
        encoding: "utf8",
        shell: false,
        ...opts,
    });
}

function gh(args, opts = {}) {
    return run("gh", args, {
        env: {
            ...process.env,
            GH_TOKEN: process.env.GH_TOKEN || process.env.GITHUB_TOKEN || "",
        },
        ...opts,
    });
}

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

function findBinary(binName) {
    const hit = walkFiles(enginesDir).find((f) => path.basename(f) === binName);
    if (!hit) throw new Error(`missing ${binName} under ${enginesDir}`);
    return hit;
}

function ensureRelease() {
    const view = gh(["release", "view", tag, "--repo", repo]);
    if (view.status === 0) {
        console.log(`release ${tag} already exists`);
        return;
    }
    console.log(`create release ${tag}`);
    const create = gh([
        "release",
        "create",
        tag,
        "--repo",
        repo,
        "--title",
        tag,
        "--generate-notes",
    ]);
    if (create.status !== 0) {
        // Race with another runner: treat existing as ok.
        const again = gh(["release", "view", tag, "--repo", repo]);
        if (again.status === 0) {
            console.log(`release ${tag} exists after create race`);
            return;
        }
        console.error(create.stderr || create.stdout);
        throw new Error(`gh release create failed for ${tag}`);
    }
}

function listAssetNames() {
    const r = gh([
        "release",
        "view",
        tag,
        "--repo",
        repo,
        "--json",
        "assets",
        "--jq",
        ".assets[].name",
    ]);
    if (r.status !== 0) return new Set();
    return new Set(
        (r.stdout || "")
            .split(/\r?\n/)
            .map((s) => s.trim())
            .filter(Boolean),
    );
}

function makeZip(pkg, bin) {
    fs.mkdirSync(zipsDir, { recursive: true });
    const src = findBinary(bin);
    const stage = path.join(zipsDir, `_stage_${pkg}`);
    fs.rmSync(stage, { recursive: true, force: true });
    fs.mkdirSync(stage, { recursive: true });
    const stagedBin = path.join(stage, bin);
    fs.copyFileSync(src, stagedBin);
    try {
        fs.chmodSync(stagedBin, 0o755);
    } catch {
        // windows
    }

    const zipName = `${pkg}.zip`;
    const zipPath = path.join(zipsDir, zipName);
    fs.rmSync(zipPath, { force: true });

    // Portable zip via bestzip-less approach: use system zip or PowerShell-free node.
    // On ubuntu-latest, `zip` is available.
    const z = run("zip", ["-j", zipPath, stagedBin], { stdio: "inherit" });
    if (z.status !== 0) {
        throw new Error(`zip failed for ${pkg}`);
    }
    fs.rmSync(stage, { recursive: true, force: true });
    return { zipName, zipPath };
}

ensureRelease();
const existing = listAssetNames();
const summary = { uploaded: [], skipped: [] };

for (const { pkg, bin } of packages) {
    const { zipName, zipPath } = makeZip(pkg, bin);
    if (existing.has(zipName)) {
        console.log(`skip asset ${zipName} (already on release ${tag})`);
        summary.skipped.push(zipName);
        continue;
    }
    console.log(`upload ${zipName}`);
    const up = gh(["release", "upload", tag, zipPath, "--repo", repo], { stdio: "inherit" });
    if (up.status !== 0) {
        const assets = listAssetNames();
        if (assets.has(zipName)) {
            console.log(`skip asset ${zipName} (exists after upload error)`);
            summary.skipped.push(zipName);
            continue;
        }
        throw new Error(`gh release upload failed for ${zipName}`);
    }
    summary.uploaded.push(zipName);
}

console.log(JSON.stringify(summary));
