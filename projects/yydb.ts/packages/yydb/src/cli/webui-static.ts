import { createRequire } from 'node:module';
import { createServer, type Server } from 'node:http';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, type ChildProcess } from 'node:child_process';

const MIME: Record<string, string> = {
    '.html': 'text/html; charset=utf-8',
    '.js': 'text/javascript; charset=utf-8',
    '.css': 'text/css; charset=utf-8',
    '.json': 'application/json; charset=utf-8',
    '.svg': 'image/svg+xml',
    '.woff2': 'font/woff2',
    '.png': 'image/png',
    '.ico': 'image/x-icon',
};

function contentType(path: string): string {
    const extension = path.slice(path.lastIndexOf('.'));
    return MIME[extension] ?? 'application/octet-stream';
}

export function resolveWebuiDist(): string {
    const bundled = join(dirname(fileURLToPath(import.meta.url)), '..', 'webui');
    if (existsSync(join(bundled, 'index.html'))) {
        return bundled;
    }
    // Workspace fallback when yydb was built with tsc only (no bundle-webui step).
    try {
        const require = createRequire(import.meta.url);
        const pkgJson = require.resolve('@yydb/yydb-webui/package.json');
        const external = join(dirname(pkgJson), 'dist', 'cdn');
        if (existsSync(join(external, 'index.html'))) {
            return external;
        }
    } catch {
        // @yydb/yydb-webui not installed
    }
    return bundled;
}

export function wsEndpointFromBind(bind: string): string {
    const hostPort = bind.includes('://') ? bind : `http://${bind}`;
    const url = new URL(hostPort);
    const wsProtocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    const path = url.pathname.endsWith('/wire') ? url.pathname : `${url.pathname.replace(/\/$/, '')}/wire`;
    return `${wsProtocol}//${url.host}${path === '/' ? '/wire' : path}`;
}

export function startStaticServer(root: string, port: number): Promise<Server> {
    const absRoot = resolve(root);
    return new Promise((resolvePromise, reject) => {
        const server = createServer((request, response) => {
            const pathname = decodeURIComponent(new URL(request.url ?? '/', 'http://localhost').pathname);
            const relative = pathname === '/' ? 'index.html' : pathname.replace(/^\//, '');
            const target = resolve(absRoot, relative);
            if (!target.startsWith(absRoot)) {
                response.writeHead(403);
                response.end('forbidden');
                return;
            }
            try {
                const bytes = readFileSync(target);
                response.writeHead(200, { 'Content-Type': contentType(target) });
                response.end(bytes);
            } catch {
                response.writeHead(404);
                response.end('not found');
            }
        });
        server.listen(port, '127.0.0.1', () => resolvePromise(server));
        server.on('error', reject);
    });
}

export function spawnServe(dbPath: string, bind: string, insecureBind: boolean): ChildProcess {
    const cliJs = fileURLToPath(new URL('../cli.js', import.meta.url));
    const args = [cliJs, 'serve', dbPath, '--bind', bind];
    if (insecureBind) args.push('--insecure-bind');
    return spawn(process.execPath, args, {
        stdio: ['ignore', 'pipe', 'pipe'],
        env: process.env,
    });
}

export function openBrowser(url: string): void {
    const platform = process.platform;
    if (platform === 'win32') {
        spawn('cmd', ['/c', 'start', '', url], { detached: true, stdio: 'ignore' });
        return;
    }
    const opener = platform === 'darwin' ? 'open' : 'xdg-open';
    spawn(opener, [url], { detached: true, stdio: 'ignore' });
}

export function sleep(ms: number): Promise<void> {
    return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

export async function waitForWire(endpoint: string, timeoutMs = 8000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const socket = new WebSocket(endpoint);
            const opened = await new Promise<boolean>((resolveOpen) => {
                const timer = setTimeout(() => resolveOpen(false), 500);
                socket.addEventListener('open', () => {
                    clearTimeout(timer);
                    resolveOpen(true);
                });
                socket.addEventListener('error', () => {
                    clearTimeout(timer);
                    resolveOpen(false);
                });
            });
            socket.close();
            if (opened) return;
        } catch {
            // Node may not expose WebSocket until a recent version; fall back to delay.
            await sleep(400);
            return;
        }
        await sleep(200);
    }
}
