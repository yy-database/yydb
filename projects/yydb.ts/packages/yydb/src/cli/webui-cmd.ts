import fs from 'node:fs';
import { join } from 'node:path';
import type { ChildProcess } from 'node:child_process';
import type { Server } from 'node:http';
import type { Cli, ParsedOptions } from '@vmz/commander';

import { flag, str } from './options.js';
import {
    openBrowser,
    resolveWebuiDist,
    spawnServe,
    startStaticServer,
    waitForWire,
    wsEndpointFromBind,
} from './webui-static.js';

function parsePort(raw: string | undefined, fallback: number): number {
    const value = Number(raw ?? fallback);
    if (!Number.isInteger(value) || value < 1 || value > 65535) {
        throw new Error(`invalid port: ${raw ?? fallback}`);
    }
    return value;
}

export function registerWebuiCommand(cli: Cli): void {
    cli.command('webui', 'cli.cmd.webui')
        .option('--bind <host:port>', 'cli.opt.bind')
        .option('--port <number>', 'cli.opt.webui-port')
        .option('--endpoint <url>', 'cli.opt.endpoint')
        .option('--no-open', 'cli.opt.no-open')
        .option('--insecure-bind', 'cli.opt.insecure-bind')
        .action((options) => cmdWebui(options));
}

export async function cmdWebui(options: ParsedOptions): Promise<number> {
    const endpointOverride = str(options, 'endpoint');
    const dbPath = options._[0] as string | undefined;
    if (!endpointOverride && !dbPath) {
        throw new Error('webui requires a database path or --endpoint');
    }

    const bind = str(options, 'bind') ?? '127.0.0.1:7700';
    const webuiPort = parsePort(str(options, 'port'), 8780);
    const endpoint = endpointOverride ?? wsEndpointFromBind(bind);
    const distRoot = resolveWebuiDist();
    if (!fs.existsSync(join(distRoot, 'index.html'))) {
        throw new Error('WebUI assets not found. Run `pnpm --filter @yydb/yydb build` to bundle dist/webui.');
    }

    let serveChild: ChildProcess | null = null;
    if (!endpointOverride && dbPath) {
        serveChild = spawnServe(dbPath, bind, flag(options, 'insecure-bind'));
        serveChild.stdout?.on('data', (chunk) => process.stdout.write(chunk));
        serveChild.stderr?.on('data', (chunk) => process.stderr.write(chunk));
        serveChild.on('exit', (code) => {
            if (code && code !== 0) {
                console.error(`yydb serve exited with code ${code}`);
            }
        });
        await waitForWire(endpoint, 8000);
    }

    const server = await startStaticServer(distRoot, webuiPort);
    const pageUrl = `http://127.0.0.1:${webuiPort}/?endpoint=${encodeURIComponent(endpoint)}`;

    console.log(`WebUI ${pageUrl}`);
    console.log(`Wire ${endpoint}`);

    if (options.open !== false) {
        openBrowser(pageUrl);
    }

    await waitForShutdown(server, serveChild);
    return 0;
}

function waitForShutdown(server: Server, serveChild: ChildProcess | null): Promise<void> {
    return new Promise((resolveWait) => {
        const shutdown = () => {
            server.close();
            if (serveChild && !serveChild.killed) {
                serveChild.kill();
            }
            resolveWait();
        };
        process.once('SIGINT', shutdown);
        process.once('SIGTERM', shutdown);
    });
}
