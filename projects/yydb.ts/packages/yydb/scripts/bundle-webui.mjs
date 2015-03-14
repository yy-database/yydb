/**
 * Build @yydb/yydb-webui and copy dist/cdn into @yydb/yydb/dist/webui for publish.
 */
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const pkgRoot = path.resolve(here, "..");
const webuiRoot = path.resolve(pkgRoot, "../yydb-webui");
const webuiDist = path.join(webuiRoot, "dist", "cdn");
const target = path.join(pkgRoot, "dist", "webui");

function ensureWebuiBuilt() {
    const index = path.join(webuiDist, "index.html");
    if (fs.existsSync(index)) return;
    const result = spawnSync("pnpm", ["run", "build"], {
        cwd: webuiRoot,
        stdio: "inherit",
        shell: process.platform === "win32",
    });
    if (result.status !== 0) {
        process.exit(result.status ?? 1);
    }
    if (!fs.existsSync(index)) {
        console.error("@yydb/yydb-webui build did not produce dist/cdn/index.html");
        process.exit(1);
    }
}

function copyDir(src, dest) {
    fs.mkdirSync(dest, { recursive: true });
    for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
        const from = path.join(src, entry.name);
        const to = path.join(dest, entry.name);
        if (entry.isDirectory()) copyDir(from, to);
        else fs.copyFileSync(from, to);
    }
}

ensureWebuiBuilt();
if (fs.existsSync(target)) fs.rmSync(target, { recursive: true, force: true });
copyDir(webuiDist, target);
console.log(`Bundled WebUI → dist/webui`);
