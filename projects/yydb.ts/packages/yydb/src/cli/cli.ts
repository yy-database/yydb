import fs from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { createCli } from '@vmz/commander';
import { formatDiagnostic } from '@vmz/diagnostic';

import { YydbError } from '../shared/errors.js';
import { registerInfoCommand } from './info-cmd.js';
import { registerInitCommand } from './init-cmd.js';
import { registerServeCommand } from './serve-cmd.js';
import { registerVerifyCommand } from './verify-cmd.js';
import { registerVersionCommand } from './version-cmd.js';

const CLI_CATALOG: Record<string, string> = JSON.parse(
    fs.readFileSync(resolve(import.meta.dirname, '../../locales/en-US/cli.json'), 'utf8'),
);

function buildYydbCli() {
    const localesRoot = join(dirname(fileURLToPath(import.meta.url)), '../../locales');
    const cli = createCli('yydb').locales(localesRoot).intro('cli.intro');

    registerVersionCommand(cli);
    registerInitCommand(cli);
    registerInfoCommand(cli);
    registerVerifyCommand(cli);
    registerServeCommand(cli);

    return cli;
}

export async function runCli(argv: string[]): Promise<number> {
    try {
        return await buildYydbCli().parse(argv);
    } catch (error) {
        const parsed = error instanceof Error
            ? YydbError.fromDiagnosticJson(error.message)
            : null;
        if (parsed) {
            // Structured engine error from N-API / wire / WASM envelope.
            const severity: 'error' | 'warning' = parsed.retryable ? 'warning' : 'error';
            const text = formatDiagnostic({
                path: '',
                severity,
                code: parsed.code,
                args: parsed.params,
                message: error instanceof Error ? error.message : String(error),
            }, {
                locale: 'en-US',
                catalog: CLI_CATALOG,
                style: 'compact',
            });
            console.error(text);
            return parsed.retryable ? 13 : 1;
        }
        // Plain text fallback (commander parse failures, unknown errors).
        console.error(`error: ${error instanceof Error ? error.message : String(error)}`);
        return 1;
    }
}
