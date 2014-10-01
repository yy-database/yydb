/**
 * CRC32 校验和
 *
 * 只依赖 `Uint8Array`，浏览器与测试环境共用同一实现。
 */

/** CRC32 查找表 */
const CRC32_TABLE: Uint32Array = buildTable();

/**
 * 构建 CRC32 查找表
 */
function buildTable(): Uint32Array {
    const table = new Uint32Array(256);
    for (let i = 0; i < 256; i++) {
        let crc = i;
        for (let j = 0; j < 8; j++) {
            crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
        }
        table[i] = crc >>> 0;
    }
    return table;
}

/**
 * 计算 CRC32
 * @param data 数据
 * @param offset 起始偏移量
 * @param length 计算长度，默认到数据末尾
 */
export function crc32(data: Uint8Array, offset: number = 0, length?: number): number {
    const end = length === undefined ? data.length : Math.min(data.length, offset + length);
    let crc = 0xffffffff;

    for (let i = offset; i < end; i++) {
        crc = CRC32_TABLE[(crc ^ data[i]!) & 0xff]! ^ (crc >>> 8);
    }

    return (crc ^ 0xffffffff) >>> 0;
}

/**
 * 比较校验和
 * @param data 数据
 * @param expected 期望的校验和
 * @param offset 起始偏移量
 * @param length 计算长度
 */
export function checksumMatches(data: Uint8Array, expected: number, offset: number = 0, length?: number): boolean {
    return crc32(data, offset, length) === expected;
}
