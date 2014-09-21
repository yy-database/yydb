import type { Cli, ParsedOptions } from '@vmz/commander';

import { loadNative } from '../native/binding.js';
import { requiredPositional } from './options.js';

export function registerInfoCommand(cli: Cli): void {
    cli.command('info', 'cli.cmd.info').action((options) => cmdInfo(options));
}

export function cmdInfo(options: ParsedOptions): number {
    const dbPath = requiredPositional(options, 'info');
    const database = loadNative().Database.open(dbPath);

    const lines = [`path=${database.path() ?? '<memory>'}`];
    const schemaVersion = database.schemaVersion();
    if (schemaVersion === null) {
        lines.push('schema=(none)');
    } else {
        lines.push(`schema.version=${schemaVersion}`);
        lines.push(`schema.document=${summarizeDocument(database.schemaDocument())}`);
    }
    lines.push(`records=${database.recordCount()}`);
    lines.push(`journal_mode=${database.journalMode()}`);
    const wal = database.walPath();
    const shm = database.shmPath();
    if (wal !== null) lines.push(`wal=${wal}`);
    if (shm !== null) lines.push(`shm=${shm}`);
    lines.push(`wal_frames=${database.walFrameCount()}`);

    console.log(lines.join('\n'));
    return 0;
}

function summarizeDocument(document: string | null): string {
    const trimmed = (document ?? '').trim();
    if (trimmed.length === 0) return '(empty)';
    const limit = 120;
    const flat = trimmed.replace(/\n/g, ' ');
    return flat.length <= limit ? flat : `${flat.slice(0, limit)}…`;
}
