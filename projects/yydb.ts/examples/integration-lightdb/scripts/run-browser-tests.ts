import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright-core";

import type { BrowserTestResult } from "../browser/runner.ts";

const root = dirname(fileURLToPath(import.meta.url));
const dist = join(root, "..", "dist");
const mime: Record<string, string> = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".wasm": "application/wasm",
    ".css": "text/css; charset=utf-8",
};

function contentType(path: string): string {
    const extension = path.slice(path.lastIndexOf("."));
    return mime[extension] ?? "application/octet-stream";
}

function createStaticServer(): Promise<{ server: ReturnType<typeof createServer>; url: string }> {
    return new Promise((resolve, reject) => {
        const server = createServer((request, response) => {
            const pathname = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
            const target = join(dist, pathname === "/" ? "index.html" : pathname);
            try {
                const bytes = readFileSync(target);
                response.writeHead(200, { "Content-Type": contentType(target) });
                response.end(bytes);
            } catch {
                response.writeHead(404);
                response.end("not found");
            }
        });
        server.listen(0, "127.0.0.1", () => {
            const address = server.address();
            if (!address || typeof address === "string") {
                reject(new Error("failed to bind static server"));
                return;
            }
            resolve({ server, url: `http://127.0.0.1:${address.port}` });
        });
    });
}

async function main(): Promise<void> {
    const { server, url } = await createStaticServer();
    const browser = await chromium.launch({
        headless: true,
        channel: process.env.YYDB_BROWSER_CHANNEL ?? "chrome",
    });
    try {
        const page = await browser.newPage();
        await page.goto(`${url}/index.html`, { waitUntil: "networkidle" });
        await page.waitForFunction(() => Array.isArray(window.__yydbLightDbResults));
        const results = await page.evaluate(() => window.__yydbLightDbResults) as BrowserTestResult[];

        const failures = results.filter((result) => !result.ok);
        for (const result of results) {
            const label = result.ok ? "ok" : "FAIL";
            console.log(`${label} ${result.name}${result.error ? `: ${result.error}` : ""}`);
        }

        if (failures.length > 0) {
            process.exitCode = 1;
        }
    } finally {
        await browser.close();
        server.close();
    }
}

await main();
