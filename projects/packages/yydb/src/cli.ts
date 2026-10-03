#!/usr/bin/env node
import { parseArgs } from "node:util";
import { loadNative } from "./native.js";

function printHelp() {
    console.log(`yydb — YYDB TypeScript CLI

Usage:
  yydb version
  yydb init <path> [--schema-version N] [--schema-file path]
  yydb info <path>
  yydb serve <path> [--bind host:port] [--insecure-bind]

Environment:
  YYDB_NATIVE  Override the platform .node binding path
`);
}

function main(): void {
    const argv = process.argv.slice(2);
    const command = argv[0];
    if (!command || command === "--help" || command === "-h") {
        printHelp();
        return;
    }

    const native = loadNative();

    switch (command) {
        case "version": {
            console.log(native.version());
            return;
        }
        case "init": {
            const { values, positionals } = parseArgs({
                args: argv.slice(1),
                options: {
                    "schema-version": { type: "string", default: "1" },
                    "schema-file": { type: "string" },
                },
                allowPositionals: true,
            });
            const dbPath = positionals[0];
            if (!dbPath) {
                throw new Error("init requires a database path");
            }
            const schemaVersion = Number.parseInt(values["schema-version"] ?? "1", 10);
            const schemaFile = values["schema-file"];
            const schemaDocument = schemaFile ? native.readSchemaFile(schemaFile) : null;
            native.initDb(dbPath, schemaVersion, schemaDocument);
            console.log(`initialized ${dbPath}`);
            return;
        }
        case "info": {
            const dbPath = argv[1];
            if (!dbPath) {
                throw new Error("info requires a database path");
            }
            console.log(native.infoText(dbPath));
            return;
        }
        case "serve": {
            const { values, positionals } = parseArgs({
                args: argv.slice(1),
                options: {
                    bind: { type: "string", default: "127.0.0.1:7700" },
                    "insecure-bind": { type: "boolean", default: false },
                },
                allowPositionals: true,
            });
            const dbPath = positionals[0];
            if (!dbPath) {
                throw new Error("serve requires a database path");
            }
            native.serve(dbPath, values.bind ?? "127.0.0.1:7700", values["insecure-bind"] ?? false);
            return;
        }
        default:
            throw new Error(`unknown command: ${command}`);
    }
}

try {
    main();
} catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error(`error: ${message}`);
    process.exitCode = 1;
}
