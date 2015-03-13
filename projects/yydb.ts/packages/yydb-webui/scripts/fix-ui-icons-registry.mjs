/**
 * @vmz/ui-icons@0.3.8 publishes `./registry` as TypeScript. VMZ dev cannot strip types
 * from node_modules. Copy precompiled JS (from vmz build vendor output) into the package.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const homepage = path.resolve(here, "..");
const vendorSrc = path.join(homepage, "vendor", "vmz", "ui-icons", "src");
const targetPkg = path.join(homepage, "node_modules", "@vmz", "ui-icons");
const targetDist = path.join(targetPkg, "dist");

if (!fs.existsSync(vendorSrc) || !fs.existsSync(targetPkg)) {
    process.exit(0);
}

fs.mkdirSync(targetDist, { recursive: true });
for (const name of fs.readdirSync(vendorSrc)) {
    if (!name.endsWith(".js")) continue;
    fs.copyFileSync(path.join(vendorSrc, name), path.join(targetDist, name));
}

const pkgPath = path.join(targetPkg, "package.json");
const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
pkg.exports = { ...(pkg.exports || {}), "./registry": "./dist/registry.js" };
const files = new Set([...(pkg.files || []), "dist"]);
pkg.files = [...files];
fs.writeFileSync(pkgPath, `${JSON.stringify(pkg, null, 2)}\n`, "utf8");
