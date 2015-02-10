import fs from 'node:fs';
import path from 'node:path';
import type { Cli, ParsedOptions } from '@vmz/commander';
import { formatDiagnostic } from '@vmz/diagnostic';

import { loadNative } from '../native/binding.js';
import type { DoctorReport } from '../api/database.js';
import { requiredPositional, str } from './options.js';

const CLI_CATALOG: Record<string, string> = JSON.parse(
    fs.readFileSync(path.resolve(import.meta.dirname, '../../locales/en-US/cli.json'), 'utf8'),
);

export function registerInfoCommand(cli: Cli): void {
    cli.command('info', 'cli.cmd.info')
        .option('--doctor', 'cli.opt.doctor')
        .action((options) => cmdInfo(options));
}

export function cmdInfo(options: ParsedOptions): number {
    const dbPath = requiredPositional(options, 'info');
    const wantDoctor = Boolean(options.doctor);

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

    if (wantDoctor) {
        const report = JSON.parse(database.doctor()) as DoctorReport;
        if (report.issues.length === 0) {
            console.log(`doctor=clean orphan_objects=${report.orphan_object_count}`);
        } else {
            for (const issue of report.issues) {
                const severity: 'error' | 'warning' = issue.severity === 'error' ? 'error' : 'warning';
                console.error(formatDiagnostic({
                    path: dbPath,
                    severity,
                    code: issue.code,
                    message: issue.message,
                }, {
                    locale: 'en-US',
                    catalog: CLI_CATALOG,
                    style: 'compact',
                }));
            }
            console.error(`doctor=issues(${report.issues.length}) orphans=${report.orphan_object_count}`);
        }
    }

    return 0;
}

function summarizeDocument(document: string | null): string {
    const trimmed = (document ?? '').trim();
    if (trimmed.length === 0) return '(empty)';
    const limit = 120;
    const flat = trimmed.replace(/\n/g, ' ');
    return flat.length <= limit ? flat : `${flat.slice(0, limit)}…`;
}
