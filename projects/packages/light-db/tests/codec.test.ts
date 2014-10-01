/**
 * codec 层测试
 */

import {
    BinaryUtils,
    ByteReader,
    ByteWriter,
    ChecksumMismatchError,
    decodeSnapshot,
    decodeWalRecord,
    encodeSnapshot,
    encodeWalRecord,
    RecordTooLargeError,
    SnapshotFormatError,
    WalOperation,
} from "../src";
import { assertEqual, assertThrows, assertTrue, describe } from "./test-utils";

/**
 * 创建 codec 测试套件
 */
export function createCodecTests() {
    const suite = describe("codec");

    suite.test("ByteWriter 与 ByteReader 往返", () => {
        const writer = new ByteWriter(8);
        writer.writeUInt8(0x7f);
        writer.writeUInt16LE(0x1234);
        writer.writeUInt32LE(0xdeadbeef);
        writer.writeBigUInt64LE(0x0102030405060708n);
        writer.writeString("星屑手记");
        writer.writeBytesWithLength(new Uint8Array([1, 2, 3]));

        const reader = new ByteReader(writer.data);
        assertEqual(reader.readUInt8(), 0x7f);
        assertEqual(reader.readUInt16LE(), 0x1234);
        assertEqual(reader.readUInt32LE(), 0xdeadbeef);
        assertEqual(reader.readBigUInt64LE(), 0x0102030405060708n);
        assertEqual(reader.readString(), "星屑手记");
        assertEqual(Array.from(reader.readBytesWithLength()), [1, 2, 3]);
        assertEqual(reader.remaining, 0, "reader should consume all bytes");
    });

    suite.test("ByteReader 校验和不匹配时抛错", async () => {
        const writer = new ByteWriter(8);
        writer.writeUInt32LE(42);
        writer.writeChecksum(0);

        const data = writer.data;
        data[0] = 43;

        const reader = new ByteReader(data);
        reader.readUInt32LE();
        await assertThrows(async () => {
            reader.readAndVerifyChecksum(0);
        }, ChecksumMismatchError);
    });

    suite.test("快照跨页往返", () => {
        const records = new Map<string, Uint8Array>();
        for (let i = 0; i < 40; i++) {
            records.set(`key-${i}`, BinaryUtils.encode(`value-${i}`));
        }

        const data = encodeSnapshot(records, 7n, 512);
        const decoded = decodeSnapshot(data);

        assertEqual(decoded.header.lastSnapshotLsn, 7n);
        assertEqual(decoded.records.size, 40);
        assertTrue(decoded.pageCount > 1, "small pages should produce multiple pages");
        assertEqual(BinaryUtils.decode(decoded.records.get("key-39")!), "value-39");
    });

    suite.test("快照拒绝损坏的页校验和", async () => {
        const records = new Map<string, Uint8Array>([["a", BinaryUtils.encode("1")]]);
        const data = encodeSnapshot(records, 1n, 512);
        data[data.length - 1] = data[data.length - 1]! ^ 0xff;

        await assertThrows(async () => {
            decodeSnapshot(data);
        }, ChecksumMismatchError);
    });

    suite.test("快照拒绝错误魔数", async () => {
        const records = new Map<string, Uint8Array>([["a", BinaryUtils.encode("1")]]);
        const data = encodeSnapshot(records, 1n, 512);
        data[0] = 0;

        await assertThrows(async () => {
            decodeSnapshot(data);
        }, SnapshotFormatError);
    });

    suite.test("超出页容量的记录被拒绝", async () => {
        const records = new Map<string, Uint8Array>([["big", new Uint8Array(600)]]);
        await assertThrows(async () => {
            encodeSnapshot(records, 1n, 512);
        }, RecordTooLargeError);
    });

    suite.test("WAL 记录往返", () => {
        const encoded = encodeWalRecord({
            lsn: 9n,
            transactionId: 3n,
            operation: WalOperation.Put,
            key: "用户",
            value: BinaryUtils.encode("value"),
        });

        const decoded = decodeWalRecord(encoded);
        assertEqual(decoded.status, "ok");
        if (decoded.status === "ok") {
            assertEqual(decoded.record.lsn, 9n);
            assertEqual(decoded.record.transactionId, 3n);
            assertEqual(decoded.record.operation, WalOperation.Put);
            assertEqual(decoded.record.key, "用户");
            assertEqual(BinaryUtils.decode(decoded.record.value!), "value");
            assertEqual(decoded.bytesRead, encoded.length);
        }
    });

    suite.test("WAL 尾部不完整时返回 incomplete", () => {
        const encoded = encodeWalRecord({
            lsn: 1n,
            transactionId: 0n,
            operation: WalOperation.Delete,
            key: "k",
            value: null,
        });

        const truncated = encoded.subarray(0, encoded.length - 3);
        assertEqual(decodeWalRecord(truncated).status, "incomplete");
    });

    return suite;
}
