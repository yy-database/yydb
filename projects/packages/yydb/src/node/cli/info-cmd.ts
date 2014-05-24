import type { Cli, ParsedOptions } from '@vmz/commander';

import { loadNative } from '../native.js';
import { requiredPositional } from './options.js';

export function registerInfoCommand(cli: Cli): void {
    cli.command('info', 'cli.cmd.info').action((options) => cmdInfo(options));
}

export function cmdInfo(options: ParsedOptions): number {
    const dbPath = requiredPositional(options, 'info');
    console.log(loadNative().infoText(dbPath));
    return 0;
}
