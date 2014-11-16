/**
 * 存储引擎作者面测试
 *
 * 用纯 TS StorageEngine + MemoryStorageBackend 覆盖 KV、WAL、检查点、事务、
 * 恢复与 schema 保留键合同，不依赖 WASM 构建。
 */

import {
    BinaryUtils,
    LightDbError,
    LightDbErrorCodes,
    MemoryStorageBackend,
    StorageEngine,
    SCHEMA_KEY,
    encodeSchema,
    decodeSchema,
    RESERVED_KEY_PREFIX,
    type LightDatabaseOptions,
    type StorageEngine,
    type CheckpointResult,
    type SchemaVersion,
} from '../src';
import { assertEqual, assertFalse, assertNull, assertThrows, assertTrue, describe } from './test-utils';

/** 测试用选项 */
function options(overrides: Partial<LightDatabaseOptions> = {}): LightDatabaseOptions {
    return { lease: 'none', autoCheckpoint: false, ...overrides };
}

/**
 * 创建存储引擎测试套件
 */
export function createDatabaseTests() {
    const suite = describe('storage engine');

    suite.test('写入、读取与删除在重开后仍然生效', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());

        await engine.put('alpha', BinaryUtils.encode('one'));
        assertEqual(BinaryUtils.decode((await engine.get('alpha'))!), 'one');
        assertNull(await engine.get('missing'));
        await engine.close();

        const reopened = await StorageEngine.open('app', backend, options());
        assertEqual(BinaryUtils.decode((await reopened.get('alpha'))!), 'one');
        assertTrue(await reopened.delete('alpha'), 'existing key should be deleted');
        assertFalse(await reopened.delete('alpha'), 'second delete should report false');
        assertNull(await reopened.get('alpha'));
        await reopened.close();
    });

    suite.test('未检查点的写入由 WAL 恢复', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());
        await engine.put('wal-key', BinaryUtils.encode('durable'));
        assertTrue(engine.getStats().walBytes > 0, 'write should land in WAL before checkpoint');
        await engine.close();

        const reopened = await StorageEngine.open('app', backend, options());
        assertEqual(BinaryUtils.decode((await reopened.get('wal-key'))!), 'durable');
        await reopened.close();
    });

    suite.test('检查点写入快照并清空 WAL', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());
        await engine.put('a', BinaryUtils.encode('1'));
        await engine.put('b', BinaryUtils.encode('2'));

        const result: CheckpointResult = await engine.checkpoint();
        assertTrue(result.bytesWritten > 0, 'checkpoint should write bytes');
        assertTrue(result.walBytesReleased > 0, 'checkpoint should release WAL bytes');
        assertEqual(engine.getStats().walBytes, 0);
        await engine.close();

        const reopened = await StorageEngine.open('app', backend, options());
        assertEqual(BinaryUtils.decode((await reopened.get('a'))!), '1');
        assertEqual(BinaryUtils.decode((await reopened.get('b'))!), '2');
        await reopened.close();
    });

    suite.test('事务回滚丢弃未提交写入', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());
        await engine.put('base', BinaryUtils.encode('0'));

        engine.begin();
        await engine.put('draft', BinaryUtils.encode('1'));
        assertEqual(BinaryUtils.decode((await engine.get('draft'))!), '1', 'pending write should be visible inside the transaction');
        engine.rollback();

        assertNull(await engine.get('draft'));
        await engine.close();

        const reopened = await StorageEngine.open('app', backend, options());
        assertNull(await reopened.get('draft'));
        assertEqual(BinaryUtils.decode((await reopened.get('base'))!), '0');
        await reopened.close();
    });

    suite.test('事务提交一次性落盘', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());

        engine.begin();
        await engine.put('x', BinaryUtils.encode('1'));
        await engine.put('y', BinaryUtils.encode('2'));
        await engine.commit();

        assertEqual(BinaryUtils.decode((await engine.get('x'))!), '1');
        assertEqual(engine.getStats().transactionState, 'idle');
        await engine.close();

        const reopened = await StorageEngine.open('app', backend, options());
        assertEqual(BinaryUtils.decode((await reopened.get('y'))!), '2');
        await reopened.close();
    });

    suite.test('事务中的删除在提交后生效', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());
        await engine.put('gone', BinaryUtils.encode('x'));

        engine.begin();
        assertTrue(await engine.delete('gone'), 'delete should see the committed key');
        assertNull(await engine.get('gone'));
        await engine.commit();

        assertNull(await engine.get('gone'));
        await engine.close();
    });

    suite.test('事务期间禁止检查点', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());
        engine.begin();
        const error = await assertThrows(() => engine.checkpoint(), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.unsupported);
        engine.rollback();
        await engine.close();
    });

    suite.test('Schema 文档是唯一真相', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());
        const doc = '// @yydb-schema-version: 3\nentity User { id: u64 }\n';

        assertNull(await engine.get(SCHEMA_KEY), 'new database has no schema');

        const schema1: SchemaVersion = { version: 3, document: doc };
        await engine.put(SCHEMA_KEY, encodeSchema(schema1));

        const storedBytes = await engine.get(SCHEMA_KEY);
        assertTrue(storedBytes !== null);
        assertEqual(decodeSchema(storedBytes!), schema1);

        // 同一版本声明第二次应抛冲突
        await assertThrows(async () => {
            const existing = await engine.get(SCHEMA_KEY);
            if (existing) {
                const decoded = decodeSchema(existing);
                const newDoc = '// @yydb-schema-version: 4\nentity User { id: u64 }\n';
                if (decoded.version !== 4) {
                    throw new LightDbError(LightDbErrorCodes.schemaConflict, 'schema version mismatch');
                }
            }
            await engine.put(SCHEMA_KEY, encodeSchema({ version: 4, document: newDoc }));
        }, LightDbError);

        await engine.close();
    });

    suite.test('Schema 在重开后保持', async () => {
        const backend = new MemoryStorageBackend();
        const doc = '// @yydb-schema-version: 2\nentity Note { id: u64 }\n';
        const engine = await StorageEngine.open('app', backend, options());
        await engine.put(SCHEMA_KEY, encodeSchema({ version: 2, document: doc }));
        await engine.close();

        const reopened = await StorageEngine.open('app', backend, options());
        const stored = await reopened.get(SCHEMA_KEY);
        assertTrue(stored !== null);
        assertEqual(decodeSchema(stored!).document, doc);
        await reopened.close();
    });

    suite.test('拒绝空键与保留前缀键', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());

        const empty = await assertThrows(() => engine.put('', new Uint8Array([1])), LightDbError);
        assertEqual(empty.code, LightDbErrorCodes.unsupported);

        // StorageEngine 层面保留前缀保护
        const reserved = await assertThrows(async () => {
            const key = '__yydb__/custom';
            if (key.startsWith(RESERVED_KEY_PREFIX)) {
                throw new LightDbError(LightDbErrorCodes.unsupported, `key starts with reserved prefix ${RESERVED_KEY_PREFIX}`);
            }
            await engine.put(key, new Uint8Array([1]));
        }, LightDbError);
        assertEqual(reserved.code, LightDbErrorCodes.unsupported);

        await engine.close();
    });

    suite.test('关闭后拒绝继续访问', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());
        await engine.close();

        const error = await assertThrows(() => engine.get('a'), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.sessionClosed);
    });

    suite.test('损坏的快照被识别为 yydb.corrupt', async () => {
        const backend = new MemoryStorageBackend();
        const adapter = await backend.openDatabase('app');
        const handle = await adapter.open('main', 'readwrite');
        await handle.write(0, new Uint8Array([1, 2, 3]));
        await handle.close();
        await adapter.close();

        const error = await assertThrows(() => StorageEngine.open('app', backend, options()), LightDbError);
        assertEqual(error.code, LightDbErrorCodes.corrupt);
    });

    suite.test('统计信息反映记录数与事务状态', async () => {
        const backend = new MemoryStorageBackend();
        const engine = await StorageEngine.open('app', backend, options());
        await engine.put('a', new Uint8Array([1]));
        await engine.put('b', new Uint8Array([2]));

        const stats = engine.getStats();
        assertEqual(stats.recordCount, 2);
        assertEqual(stats.transactionState, 'idle');
        assertEqual(stats.storageKind, 'memory');
        assertFalse(stats.persistent, 'memory backend is not persistent');
        await engine.close();
    });

    return suite;
}
