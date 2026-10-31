// WASM runtime smoke — open memory session, exercise doctor + prepare + Statement.
// Runs via vitest (`pnpm test`) — Node 17+ ESM, loads WASM from disk as ArrayBuffer
// (no `fetch` needed; wasm-pack --target web default needs one in browsers).
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { describe, beforeAll, afterAll, it, expect } from 'vitest';
import { initWasm, createMemorySession, type WasmSession } from '@yydb/yydb-unknown-wasm32';

let wasmReady = false;
let sess: WasmSession;

const USER_SCHEMA = `
table User {
    @@user_id: uuid,
    @user_name: utf8,
    active: bool,
}
`;

// ---------------------------------------------------------------------------
// WASM init + teardown (once for all tests)
// ---------------------------------------------------------------------------

beforeAll(async () => {
    if (!wasmReady) {
        const __dirname = dirname(fileURLToPath(import.meta.url));
        const wasmBytes = readFileSync(
            join(__dirname, '..', '..', '..', 'platforms', 'yydb-unknown-wasm32', 'lib', 'yydb_wasm_bg.wasm'),
        );
        await initWasm({ module: new Uint8Array(wasmBytes) });
        wasmReady = true;
    }
    sess = createMemorySession();
    sess.ensureSchema(USER_SCHEMA);
});

afterAll(() => {
    sess.close();
});

// ---------------------------------------------------------------------------
// doctorJson — WASM timestamp shim + report shape
// ---------------------------------------------------------------------------

describe('doctorJson', () => {
    it('returns ok with the expected DoctorReport keys on a fresh store', () => {
        const raw = JSON.parse(sess.doctorJson()) as Record<string, unknown>;
        expect(raw.ok).toBe(true);
        expect(raw).toHaveProperty('report');
        const report = raw.report as Record<string, unknown>;
        expect(report).toHaveProperty('last_commit_sequence');
        expect(report).toHaveProperty('last_checkpoint_sequence');
        expect(report).toHaveProperty('checkpoint_lag_records');
        expect(report).toHaveProperty('orphan_object_count');
        expect(report).toHaveProperty('namespaces');
        expect(report).toHaveProperty('issues');
    });

    it('does not panic when called (wasm SystemTime shim)', () => {
        // Regression: before the lease::now_millis cfg shim, doctor.rs called
        // SystemTime::now() which panics in browser WASM targets.
        expect(() => sess.doctorJson()).not.toThrow();
    });
});

// ---------------------------------------------------------------------------
// prepareJson / queryPreparedJson / closeStatement
// ---------------------------------------------------------------------------

describe('prepareJson + queryPreparedJson', () => {
    const insertSql =
        `User { user_id: "11111111-1111-1111-1111-111111111111", user_name: "ada", active: true }.insert()`;
    const filterSql = `User.filter(x => x.active == true).collect()`;

    it('prepareJson returns StatementInfo with the expected keys', () => {
        const raw = JSON.parse(sess.prepareJson(filterSql)) as Record<string, unknown>;
        expect(raw.ok).toBe(true);
        expect(raw.statementId).toMatch(/^stmt-\d+$/);
        expect(raw.queryKind).toBe('Read');
        expect(raw.parameters).toEqual([]);
        expect(raw.readSet).toEqual(['User']);
        expect(raw.writeSet).toEqual([]);
        expect(raw.sourceFingerprint).toBeTruthy();
        expect(raw.source).toContain('User');
    });

    it('queryPreparedJson executes the cached plan and returns typed rows', () => {
        sess.executeVosWrite(insertSql);
        const prepared = JSON.parse(sess.prepareJson(filterSql));
        const id = prepared.statementId;

        const raw = JSON.parse(sess.queryPreparedJson(JSON.stringify({ statementId: id, params: null })));
        expect(raw.ok).toBe(true);
        expect(raw.rows).toBeInstanceOf(Array);
        expect(raw.rows.length).toBeGreaterThanOrEqual(1);
        const first = raw.rows[0] as Record<string, unknown>;
        expect(first.user_name).toBe('ada');
        expect(first.active).toBe(true);
        expect(first.user_id).toBeTruthy();
    });

    it('closeStatement releases the plan; subsequent queries refuse', () => {
        const prepared = JSON.parse(sess.prepareJson(filterSql));
        const id = prepared.statementId as string;

        const closed = sess.closeStatement(id);
        expect(closed).toBe(true);

        const retry = JSON.parse(
            sess.queryPreparedJson(JSON.stringify({ statementId: id, params: null })),
        );
        expect(retry.ok).toBe(false);
        expect(String(retry.error)).toMatch(/not found/i);
    });
});
