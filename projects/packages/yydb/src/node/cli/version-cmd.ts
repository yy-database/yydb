import type { Cli } from '@vmz/commander';

import { loadNative } from '../native.js';

export function registerVersionCommand(cli: Cli): void {
    cli.command('version', 'cli.cmd.version').action(() => cmdVersion());
}

export function cmdVersion(): number {
    console.log(loadNative().version());
    return 0;
}
