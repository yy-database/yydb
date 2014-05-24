import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { createCli } from '@vmz/commander';

import { registerInfoCommand } from './info-cmd.js';
import { registerInitCommand } from './init-cmd.js';
import { registerServeCommand } from './serve-cmd.js';
import { registerVersionCommand } from './version-cmd.js';

function buildYydbCli() {
    const localesRoot = join(dirname(fileURLToPath(import.meta.url)), '../../../locales');
    const cli = createCli('yydb').locales(localesRoot).intro('cli.intro');

    registerVersionCommand(cli);
    registerInitCommand(cli);
    registerInfoCommand(cli);
    registerServeCommand(cli);

    return cli;
}

export async function runCli(argv: string[]): Promise<number> {
    try {
        return await buildYydbCli().parse(argv);
    } catch (error) {
        console.error(`error: ${error instanceof Error ? error.message : String(error)}`);
        return 1;
    }
}
