/**
 * Configure npm Trusted Publisher via registry API for @yydb packages.
 *
 *   npm login --auth-type=web
 *   node scripts/configure-trusted-publishers.mjs
 *
 * If npm returns a web OTP URL, it is opened in the browser (never printed —
 * terminal redaction turns real URLs into literal *** which 404).
 */
import fs from "node:fs";
import os from "os";
import path from "path";
import https from "https";
import { spawnSync } from "node:child_process";

const repo = "yy-database/yydb.rs";
const workflowFile = "release-npm.yml";
const environment = "NPM_PUBLISH";
const packages = [
    "@yydb/yydb-win32-x64",
    "@yydb/yydb-linux-x64",
    "@yydb/yydb-darwin-x64",
    "@yydb/yydb-darwin-arm64",
    "@yydb/yydb-client",
    "@yydb/yydb",
];

function readToken() {
    const npmrc = fs.readFileSync(path.join(os.homedir(), ".npmrc"), "utf8");
    const m = npmrc.match(/\/\/registry\.npmjs\.org\/:_authToken=(.+)/);
    if (!m) throw new Error("no registry.npmjs.org auth token; run npm login --auth-type=web");
    return m[1].trim();
}

function request(method, urlPath, { token, body, otp } = {}) {
    return new Promise((resolve, reject) => {
        const data = body ? JSON.stringify(body) : null;
        const headers = {
            accept: "application/json",
            authorization: `Bearer ${token}`,
        };
        if (otp) headers["npm-otp"] = otp;
        if (data) {
            headers["content-type"] = "application/json";
            headers["content-length"] = Buffer.byteLength(data);
        }
        const req = https.request(
            {
                method,
                hostname: "registry.npmjs.org",
                path: urlPath,
                headers,
            },
            (res) => {
                let buf = "";
                res.on("data", (c) => (buf += c));
                res.on("end", () => {
                    let json = null;
                    try {
                        json = buf ? JSON.parse(buf) : null;
                    } catch {
                        json = null;
                    }
                    resolve({ status: res.statusCode, headers: res.headers, body: buf, json });
                });
            },
        );
        req.on("error", reject);
        if (data) req.write(data);
        req.end();
    });
}

function openUrl(url) {
    if (process.platform === "win32") {
        spawnSync("cmd", ["/c", "start", "", url], { shell: false });
    } else if (process.platform === "darwin") {
        spawnSync("open", [url]);
    } else {
        spawnSync("xdg-open", [url]);
    }
}

function sleep(ms) {
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
}

function extractAuthUrl(text) {
    const m = String(text || "").match(
        /https:\/\/www\.npmjs\.com\/(?:auth\/cli|login\?next=)[^\s"'<>]+/,
    );
    return m && !m[0].includes("***") ? m[0] : null;
}

function trustBody() {
    // Registry expects an array of configs (see npm OpenAPI samples).
    return [
        {
            type: "github",
            claims: {
                repository: repo,
                workflow_ref: { file: workflowFile },
                environment,
            },
            permissions: ["createPackage"],
        },
    ];
}

const token = readToken();
console.log("token ok, configuring", packages.length, "packages");

for (const pkg of packages) {
    const enc = encodeURIComponent(pkg);
    const urlPath = `/-/package/${enc}/trust`;
    console.log(`\n=== ${pkg} ===`);

    for (let attempt = 1; attempt <= 5; attempt++) {
        // Try list first (idempotent check)
        const listed = await request("GET", urlPath, { token });
        if (listed.status === 200 && Array.isArray(listed.json) && listed.json.length) {
            const hit = listed.json.find(
                (x) =>
                    x?.type === "github" &&
                    x?.claims?.repository === repo &&
                    (x?.claims?.workflow_ref?.file || x?.claims?.workflowFilename) === workflowFile,
            );
            if (hit) {
                console.log("already configured");
                break;
            }
        }

        const res = await request("POST", urlPath, { token, body: trustBody() });
        if (res.status === 200 || res.status === 201) {
            console.log("configured");
            break;
        }

        const authUrl =
            extractAuthUrl(res.body) ||
            extractAuthUrl(JSON.stringify(res.json || {})) ||
            extractAuthUrl(res.headers?.location || "");

        if (res.status === 401 || res.status === 403) {
            console.log(
                "status",
                res.status,
                (res.json && res.json.message) || res.body.slice(0, 200),
            );
            if (authUrl) {
                console.log(
                    "Opened web OTP. Finish in browser (skip 2FA for 5 minutes if offered), waiting…",
                );
                openUrl(authUrl);
                sleep(40000);
                continue;
            }
            // Some responses put done URL only
            const done = String(res.body).match(
                /https:\/\/registry\.npmjs\.org\/-\/v1\/done\?authId=[^\s"'<>]+/,
            );
            if (done && !done[0].includes("***")) {
                console.log("Opened registry done URL, waiting…");
                openUrl(done[0]);
                sleep(40000);
                continue;
            }
            console.error(
                "Need interactive 2FA that this environment cannot surface.\n" +
                    "Configure manually on each package Settings → Trusted Publisher:\n" +
                    `  GitHub Actions / repo ${repo} / file ${workflowFile} / env ${environment}`,
            );
            for (const p of packages) {
                console.error(`  https://www.npmjs.com/package/${p}/access`);
            }
            process.exit(1);
        }

        if (res.status === 409) {
            console.log("already exists (409)");
            break;
        }

        console.error("unexpected", res.status, res.body.slice(0, 500));
        process.exit(1);
    }
    sleep(1500);
}

console.log("\n=== verify ===");
for (const pkg of packages) {
    const enc = encodeURIComponent(pkg);
    const res = await request("GET", `/-/package/${enc}/trust`, { token });
    const ok =
        res.status === 200 && Array.isArray(res.json) && res.json.some((x) => x?.type === "github");
    console.log(ok ? "ok" : "MISSING", pkg, res.status);
}
