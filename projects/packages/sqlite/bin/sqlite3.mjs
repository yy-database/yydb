#!/usr/bin/env node
import { readFileSync } from "node:fs";

import { SqliteDatabase, SqliteSnapshot } from "../src/node.ts";

const FORMAT_VERSION = "3.45.0";

function runDotCommand(database, command) {
    if (command === ".tables") {
        const tables = database
            .schema()
            .filter((object) => (object.kind === "table" || object.kind === "view") && !object.name.startsWith("sqlite_"))
            .map((object) => object.name)
            .sort();
        for (const name of tables) console.log(name);
        return;
    }
    if (command === ".schema") {
        const statements = database
            .schema()
            .filter((object) => object.sql && !object.name.startsWith("sqlite_"))
            .map((object) => object.sql?.trimEnd() ?? "");
        for (const source of statements) {
            console.log(source.endsWith(";") ? source : `${source};`);
        }
        return;
    }
    if (command === ".ping") {
        const status = database.ping();
        console.log(`ok ${status}`);
        return;
    }
    if (command === ".encoding") {
        console.log(database.encoding());
        return;
    }
    if (command === ".version") {
        console.log(database.sqliteVersion());
        return;
    }
    const dump = command.match(/^\.dump\s+(\S+)\s*$/);
    if (dump) {
        const table = dump[1];
        const rows = database.table(table);
        if (rows.length === 0) {
            return;
        }
        const width = new Array(rows[0].columns.length).fill(0);
        for (const row of rows) {
            for (let i = 0; i < row.columns.length; i++) {
                const text = formatValue(row.columns[i]);
                width[i] = Math.max(width[i], text.length);
            }
        }
        const line = (cells) => cells.map((c, i) => c.padEnd(width[i])).join(" | ");
        for (const row of rows) {
            console.log(
                line([
                    String(row.rowid),
                    ...row.columns.map(formatValue),
                ]),
            );
        }
        return;
    }
    throw new Error(`unsupported dot command '${command}'`);
}

function formatValue(value) {
    if (value.kind === "null") return "NULL";
    if (value.kind === "integer") return String(value.valueInteger ?? "");
    if (value.kind === "real") {
        const n = value.valueReal;
        return n === undefined ? "" : (Number.isInteger(n) ? `${n}.0` : String(n));
    }
    if (value.kind === "text") return value.valueText ?? "";
    if (value.kind === "blob") {
        const buf = value.valueBlob;
        if (!buf) return "";
        let hex = "";
        for (let i = 0; i < Math.min(buf.length, 64); i++) {
            hex += buf[i].toString(16).padStart(2, "0");
        }
        return `X'${hex}${buf.length > 64 ? "…" : ""}'`;
    }
    return "";
}

function main(args) {
    if (args.length === 1 && ["--version", "-version"].includes(args[0])) {
        console.log(`sqlite3 ${FORMAT_VERSION} (YYDS pure-Rust format-3 reader)`);
        return;
    }
    if (args.length === 1 && ["--help", "-help"].includes(args[0])) {
        console.log(
            "Usage: sqlite3 DATABASE COMMAND\n" +
                "Supported dot commands: .tables, .schema, .ping, .encoding, .version, .dump TABLE\n" +
                "This CLI inspects on-disk main-file bytes only; SQL execution is not available.",
        );
        return;
    }
    if (args.length !== 2 || args[0].startsWith("-") || args[0] === "") {
        throw new Error("expected DATABASE and one supported dot command");
    }
    const [path, input] = args;
    const command = input.trim();
    if (!command.startsWith(".")) {
        throw new Error("SQL execution is not available; use a dot command");
    }
    const database = SqliteDatabase.open(path);
    runDotCommand(database, command);
}

try {
    main(process.argv.slice(2));
} catch (error) {
    console.error(`@yydb/sqlite: ${error.message}`);
    process.exitCode = 1;
}
