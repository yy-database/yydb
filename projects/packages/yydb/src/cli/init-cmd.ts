import type { Cli, ParsedOptions } from "@vmz/commander";

import { loadNative } from "../native.js";
import { requiredPositional, str } from "./options.js";

export function registerInitCommand(cli: Cli): void {
    cli.command("init", "cli.cmd.init")
        .option("--schema-version <n>", "cli.opt.schema-version")
        .option("--schema-file <path>", "cli.opt.schema-file")
        .action((options) => cmdInit(options));
}

export function cmdInit(options: ParsedOptions): number {
    const dbPath = requiredPositional(options, "init");
    const schemaVersion = Number.parseInt(str(options, "schema-version") ?? "1", 10);
    const schemaFile = str(options, "schema-file");
    const native = loadNative();
    const schemaDocument = schemaFile ? native.readSchemaFile(schemaFile) : null;
    native.initDb(dbPath, schemaVersion, schemaDocument);
    console.log(`initialized ${dbPath}`);
    return 0;
}
