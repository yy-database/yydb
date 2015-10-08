// Binary shape of the `__yydb/v0/row/*` values written by the query codec.
// Tests read the stored tags here to assert placement decisions on the bytes
// the database actually keeps, not on a JSON projection of them.

export const ROW_CODEC_VERSION = 1;

export const TAG_BOOL = 0x01;
export const TAG_I64 = 0x02;
export const TAG_F64 = 0x03;
export const TAG_TEXT = 0x04;
export const TAG_UUID = 0x05;
export const TAG_DATE = 0x06;
export const TAG_TIME = 0x07;
export const TAG_TIMESTAMP = 0x08;
export const TAG_DECIMAL = 0x09;
export const TAG_BYTES = 0x0a;
export const TAG_VECTOR = 0x0b;
export const TAG_OBJECT = 0x0c;
export const TAG_FILE = 0x0d;

/** One stored column: its value tag and the payload bytes that follow it. */
export interface StoredColumn {
    readonly tag: number;
    readonly bytes: Buffer;
}

/**
 * Decodes the column names and value tags of one stored row.
 *
 * The layout is `version | u16 column count | null bitmap | names | values`,
 * where each name is `u16 length + UTF-8` and each non-null value is a tag
 * byte plus a payload sized by that tag. `bytes` holds the value content, so
 * the `u16` length prefix of text and byte payloads is stripped.
 */
export function decodeStoredRow(row: Uint8Array): Map<string, StoredColumn> {
    const bytes = Buffer.from(row);
    const version = bytes[0];
    if (version !== ROW_CODEC_VERSION) {
        throw new Error(`stored row uses codec version ${version}, expected ${ROW_CODEC_VERSION}`);
    }
    let offset = 1;
    const columns = bytes.readUInt16LE(offset);
    offset += 2;
    const bitmapLength = Math.ceil(columns / 8);
    const bitmap = bytes.subarray(offset, offset + bitmapLength);
    offset += bitmapLength;

    const names: string[] = [];
    for (let index = 0; index < columns; index += 1) {
        const length = bytes.readUInt16LE(offset);
        offset += 2;
        names.push(bytes.toString("utf8", offset, offset + length));
        offset += length;
    }

    const decoded = new Map<string, StoredColumn>();
    for (let index = 0; index < columns; index += 1) {
        const name = names[index];
        if ((bitmap[Math.floor(index / 8)] & (1 << (index % 8))) !== 0) {
            decoded.set(name, { tag: 0, bytes: Buffer.alloc(0) });
            continue;
        }
        const tag = bytes[offset];
        offset += 1;
        const { size, prefix } = valueLayout(bytes, offset, tag);
        decoded.set(name, { tag, bytes: bytes.subarray(offset + prefix, offset + size) });
        offset += size;
    }
    return decoded;
}

function valueLayout(bytes: Buffer, offset: number, tag: number): { size: number; prefix: number } {
    switch (tag) {
        case TAG_BOOL:
            return { size: 1, prefix: 0 };
        case TAG_I64:
        case TAG_F64:
        case TAG_TIME:
        case TAG_TIMESTAMP:
            return { size: 8, prefix: 0 };
        case TAG_DATE:
            return { size: 4, prefix: 0 };
        case TAG_UUID:
            return { size: 16, prefix: 0 };
        case TAG_DECIMAL:
            return { size: 18, prefix: 0 };
        case TAG_TEXT:
        case TAG_BYTES:
            return { size: 2 + bytes.readUInt16LE(offset), prefix: 2 };
        case TAG_VECTOR:
            return { size: 2 + 4 + bytes.readUInt32LE(offset + 2), prefix: 0 };
        case TAG_OBJECT:
            return { size: 42, prefix: 0 };
        case TAG_FILE: {
            const flags = bytes[offset + 40];
            let cursor = offset + 41;
            if ((flags & 0b01) !== 0) {
                cursor += 2 + bytes.readUInt16LE(cursor);
            }
            if ((flags & 0b10) !== 0) {
                cursor += 2 + bytes.readUInt16LE(cursor);
            }
            return { size: cursor - offset + 40, prefix: 0 };
        }
        default:
            throw new Error(`stored row carries unknown value tag ${tag}`);
    }
}
