import { SqliteDatabase } from "@yydb/sqlite";

import { CASE_DDL, PAGE_SIZE, SAMPLES_DDL } from "./shared.ts";

function pushVarint(out: number[], value: number | bigint): void {
    let rest = BigInt(value);
    if (rest > 0x00ff_ffff_ffff_ffffn) {
        const bytes = new Array<number>(9);
        for (let index = 0; index < 8; index += 1) {
            bytes[index] = Number(((rest >> BigInt(57 - 7 * index)) & 0x7fn) | 0x80n);
        }
        bytes[8] = Number(rest & 0xffn);
        out.push(...bytes);
        return;
    }
    const bytes: number[] = [];
    bytes.push(Number(rest & 0x7fn));
    rest >>= 7n;
    while (rest > 0n) {
        bytes.unshift(Number((rest & 0x7fn) | 0x80n));
        rest >>= 7n;
    }
    out.push(...bytes);
}

export function encodeRecord(serials: number[], body: Buffer): Buffer {
    const header: number[] = [];
    for (const serial of serials) {
        pushVarint(header, serial);
    }
    const payload: number[] = [];
    pushVarint(payload, header.length + 1);
    payload.push(...header, ...body);
    return Buffer.from(payload);
}

export function encodeCell(payload: Buffer, rowid: number): Buffer {
    const bytes: number[] = [];
    pushVarint(bytes, payload.length);
    pushVarint(bytes, rowid);
    return Buffer.concat([Buffer.from(bytes), payload]);
}

function writeU16(page: Buffer, offset: number, value: number): void {
    page.writeUInt16BE(value, offset);
}

function writeU32(page: Buffer, offset: number, value: number): void {
    page.writeUInt32BE(value, offset);
}

function writeLeaf(page: Buffer, header: number, cells: Buffer[]): void {
    page.fill(0, header);
    page[header] = 13;
    writeU16(page, header + 3, cells.length);
    let content = page.length;
    let pointer = header + 8;
    for (const cell of cells) {
        content -= cell.length;
        cell.copy(page, content);
        writeU16(page, pointer, content);
        pointer += 2;
    }
    writeU16(page, header + 5, content);
}

function setEncoding(pages: Buffer, encoding: number): void {
    writeU32(pages, 56, encoding);
}

function blankPages(pageCount: number): Buffer {
    const blank = SqliteDatabase.openInMemory().pages();
    const pages = Buffer.alloc(PAGE_SIZE * pageCount);
    blank.copy(pages, 0, 0, PAGE_SIZE);
    if (pageCount > 1) {
        writeU32(pages, 28, pageCount);
    }
    setEncoding(pages, 1);
    return pages;
}

function schemaRecord(
    kind: string,
    name: string,
    tableName: string,
    rootPage: number | null,
    sql: string | null,
): Buffer {
    const serials: number[] = [];
    const bodyParts: Buffer[] = [];
    for (const text of [kind, name, tableName]) {
        const bytes = Buffer.from(text, "utf8");
        serials.push(13 + 2 * bytes.length);
        bodyParts.push(bytes);
    }
    if (rootPage === null) {
        serials.push(0);
    } else {
        serials.push(1);
        bodyParts.push(Buffer.from([rootPage]));
    }
    if (sql === null) {
        serials.push(0);
    } else {
        const sqlBytes = Buffer.from(sql, "utf8");
        serials.push(13 + 2 * sqlBytes.length);
        bodyParts.push(sqlBytes);
    }
    return encodeRecord(serials, Buffer.concat(bodyParts));
}

function caseRowPayload(label: string): Buffer {
    const text = Buffer.from(label, "utf8");
    return encodeRecord([0, 13 + 2 * text.byteLength], text);
}

export interface CaseRow {
    rowid: number;
    label: string;
}

/**
 * Materializes a two-page format-3 file: page 1 holds `sqlite_master`, page 2
 * stores the `案例` rowid table. This is YYDS-side creation without SQL.
 */
export function buildCaseDatabase(rows: readonly CaseRow[]): Buffer {
    const schemaCell = encodeCell(
        schemaRecord("table", "案例", "案例", 2, CASE_DDL),
        1,
    );
    const dataCells = rows.map((row) => encodeCell(caseRowPayload(row.label), row.rowid));
    const pages = blankPages(2);
    writeLeaf(pages.subarray(0, PAGE_SIZE), 100, [schemaCell]);
    writeLeaf(pages.subarray(PAGE_SIZE), 0, dataCells);
    return pages;
}

export interface SamplesRow {
    rowid: number;
    payload: Buffer;
}

/**
 * Materializes the shared `samples` table using on-disk payloads that match
 * `node:sqlite` so Node can SELECT the same storage classes back.
 */
export function buildSamplesDatabase(rows: readonly SamplesRow[]): Buffer {
    const schemaCell = encodeCell(
        schemaRecord("table", "samples", "samples", 2, SAMPLES_DDL),
        1,
    );
    const dataCells = rows.map((row) => encodeCell(row.payload, row.rowid));
    const pages = blankPages(2);
    writeLeaf(pages.subarray(0, PAGE_SIZE), 100, [schemaCell]);
    writeLeaf(pages.subarray(PAGE_SIZE), 0, dataCells);
    return pages;
}
