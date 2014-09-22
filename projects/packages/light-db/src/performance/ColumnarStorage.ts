/**
 * 列式存储实现
 * 使用平行数组存储数据，降低 GC 压力，提高聚合和范围扫描性能
 */

import {
    ColumnType,
    ColumnDefinition,
    ColumnarStorageOptions,
    ColumnarStorageStats,
    ColumnStats,
    AggregationType,
    AggregationResult,
    RangeCondition,
} from "./types";

/**
 * 列数据接口
 */
interface IColumn {
    readonly name: string;
    readonly type: ColumnType;
    readonly length: number;
    get(index: number): unknown;
    set(index: number, value: unknown): void;
    push(value: unknown): void;
    fill(value: unknown, start: number, end: number): void;
    getMemoryUsage(): number;
    getNonNullCount(): number;
    grow(newCapacity: number): void;
}

/**
 * 类型化数组列实现
 */
abstract class TypedArrayColumn<T extends ArrayBufferView> implements IColumn {
    protected data: T;
    protected nullBitmap: Uint8Array;
    protected _length: number;
    protected _capacity: number;
    public readonly name: string;
    public readonly type: ColumnType;

    constructor(name: string, type: ColumnType, capacity: number, createArray: (size: number) => T) {
        this.name = name;
        this.type = type;
        this._capacity = capacity;
        this._length = 0;
        this.data = createArray(capacity);
        this.nullBitmap = new Uint8Array(Math.ceil(capacity / 8));
    }

    public get length(): number {
        return this._length;
    }

    public getData(): T {
        return this.data;
    }

    public get(index: number): unknown {
        if (index < 0 || index >= this._length) {
            return undefined;
        }
        if (this.isNull(index)) {
            return null;
        }
        return this.getValue(index);
    }

    public set(index: number, value: unknown): void {
        if (index < 0 || index >= this._capacity) {
            throw new RangeError(`Index ${index} out of bounds`);
        }
        if (value === null || value === undefined) {
            this.setNull(index, true);
        } else {
            this.setNull(index, false);
            this.setValue(index, value);
        }
        if (index >= this._length) {
            this._length = index + 1;
        }
    }

    public push(value: unknown): void {
        if (this._length >= this._capacity) {
            throw new Error("Column is full, call grow() first");
        }
        this.set(this._length, value);
    }

    public fill(value: unknown, start: number, end: number): void {
        for (let i = start; i < end; i++) {
            this.set(i, value);
        }
    }

    protected isNull(index: number): boolean {
        const byteIndex = Math.floor(index / 8);
        const bitIndex = index % 8;
        return (this.nullBitmap[byteIndex]! & (1 << bitIndex)) !== 0;
    }

    protected setNull(index: number, isNull: boolean): void {
        const byteIndex = Math.floor(index / 8);
        const bitIndex = index % 8;
        if (isNull) {
            this.nullBitmap[byteIndex]! |= 1 << bitIndex;
        } else {
            this.nullBitmap[byteIndex]! &= ~(1 << bitIndex);
        }
    }

    public getMemoryUsage(): number {
        return this.data.byteLength + this.nullBitmap.byteLength + 8;
    }

    public getNonNullCount(): number {
        let count = 0;
        for (let i = 0; i < this._length; i++) {
            if (!this.isNull(i)) {
                count++;
            }
        }
        return count;
    }

    public grow(newCapacity: number): void {
        if (newCapacity <= this._capacity) {
            return;
        }
        const newData = this.createArray(newCapacity);
        (newData as unknown as Uint8Array).set(this.data as unknown as Uint8Array);
        this.data = newData;

        const newNullBitmap = new Uint8Array(Math.ceil(newCapacity / 8));
        newNullBitmap.set(this.nullBitmap);
        this.nullBitmap = newNullBitmap;

        this._capacity = newCapacity;
    }

    protected abstract getValue(index: number): unknown;
    protected abstract setValue(index: number, value: unknown): void;
    protected abstract createArray(size: number): T;
}

/**
 * Int8 列
 */
class Int8Column extends TypedArrayColumn<Int8Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.Int8, capacity, (s) => new Int8Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = value as number;
    }

    protected createArray(size: number): Int8Array {
        return new Int8Array(size);
    }
}

/**
 * Int16 列
 */
class Int16Column extends TypedArrayColumn<Int16Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.Int16, capacity, (s) => new Int16Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = value as number;
    }

    protected createArray(size: number): Int16Array {
        return new Int16Array(size);
    }
}

/**
 * Int32 列
 */
class Int32Column extends TypedArrayColumn<Int32Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.Int32, capacity, (s) => new Int32Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = value as number;
    }

    protected createArray(size: number): Int32Array {
        return new Int32Array(size);
    }
}

/**
 * Int64 列（使用 BigInt64Array）
 */
class Int64Column extends TypedArrayColumn<BigInt64Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.Int64, capacity, (s) => new BigInt64Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = BigInt(value as number | bigint);
    }

    protected createArray(size: number): BigInt64Array {
        return new BigInt64Array(size);
    }
}

/**
 * UInt8 列
 */
class UInt8Column extends TypedArrayColumn<Uint8Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.UInt8, capacity, (s) => new Uint8Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = value as number;
    }

    protected createArray(size: number): Uint8Array {
        return new Uint8Array(size);
    }
}

/**
 * UInt16 列
 */
class UInt16Column extends TypedArrayColumn<Uint16Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.UInt16, capacity, (s) => new Uint16Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = value as number;
    }

    protected createArray(size: number): Uint16Array {
        return new Uint16Array(size);
    }
}

/**
 * UInt32 列
 */
class UInt32Column extends TypedArrayColumn<Uint32Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.UInt32, capacity, (s) => new Uint32Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = value as number;
    }

    protected createArray(size: number): Uint32Array {
        return new Uint32Array(size);
    }
}

/**
 * UInt64 列
 */
class UInt64Column extends TypedArrayColumn<BigUint64Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.UInt64, capacity, (s) => new BigUint64Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = BigInt(value as number | bigint);
    }

    protected createArray(size: number): BigUint64Array {
        return new BigUint64Array(size);
    }
}

/**
 * Float32 列
 */
class Float32Column extends TypedArrayColumn<Float32Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.Float32, capacity, (s) => new Float32Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = value as number;
    }

    protected createArray(size: number): Float32Array {
        return new Float32Array(size);
    }
}

/**
 * Float64 列
 */
class Float64Column extends TypedArrayColumn<Float64Array> {
    constructor(name: string, capacity: number) {
        super(name, ColumnType.Float64, capacity, (s) => new Float64Array(s));
    }

    protected getValue(index: number): unknown {
        return this.data[index];
    }

    protected setValue(index: number, value: unknown): void {
        this.data[index] = value as number;
    }

    protected createArray(size: number): Float64Array {
        return new Float64Array(size);
    }
}

/**
 * 字符串列
 */
class StringColumn implements IColumn {
    public readonly name: string;
    public readonly type: ColumnType = ColumnType.String;
    private data: string[];
    private nullBitmap: Uint8Array;
    private _length: number;
    private _capacity: number;

    constructor(name: string, capacity: number) {
        this.name = name;
        this._capacity = capacity;
        this._length = 0;
        this.data = new Array(capacity);
        this.nullBitmap = new Uint8Array(Math.ceil(capacity / 8));
    }

    public get length(): number {
        return this._length;
    }

    public get(index: number): unknown {
        if (index < 0 || index >= this._length) {
            return undefined;
        }
        if (this.isNull(index)) {
            return null;
        }
        return this.data[index];
    }

    public set(index: number, value: unknown): void {
        if (index < 0 || index >= this._capacity) {
            throw new RangeError(`Index ${index} out of bounds`);
        }
        if (value === null || value === undefined) {
            this.setNull(index, true);
            this.data[index] = "";
        } else {
            this.setNull(index, false);
            this.data[index] = String(value);
        }
        if (index >= this._length) {
            this._length = index + 1;
        }
    }

    public push(value: unknown): void {
        if (this._length >= this._capacity) {
            throw new Error("Column is full, call grow() first");
        }
        this.set(this._length, value);
    }

    public fill(value: unknown, start: number, end: number): void {
        for (let i = start; i < end; i++) {
            this.set(i, value);
        }
    }

    public getMemoryUsage(): number {
        let total = this.nullBitmap.byteLength + 8;
        for (let i = 0; i < this._length; i++) {
            if (!this.isNull(i) && this.data[i]) {
                total += this.data[i]!.length * 2;
            }
        }
        return total;
    }

    public getNonNullCount(): number {
        let count = 0;
        for (let i = 0; i < this._length; i++) {
            if (!this.isNull(i)) {
                count++;
            }
        }
        return count;
    }

    public grow(newCapacity: number): void {
        if (newCapacity <= this._capacity) {
            return;
        }
        this.data.length = newCapacity;

        const newNullBitmap = new Uint8Array(Math.ceil(newCapacity / 8));
        newNullBitmap.set(this.nullBitmap);
        this.nullBitmap = newNullBitmap;

        this._capacity = newCapacity;
    }

    private isNull(index: number): boolean {
        const byteIndex = Math.floor(index / 8);
        const bitIndex = index % 8;
        return (this.nullBitmap[byteIndex]! & (1 << bitIndex)) !== 0;
    }

    private setNull(index: number, isNull: boolean): void {
        const byteIndex = Math.floor(index / 8);
        const bitIndex = index % 8;
        if (isNull) {
            this.nullBitmap[byteIndex]! |= 1 << bitIndex;
        } else {
            this.nullBitmap[byteIndex]! &= ~(1 << bitIndex);
        }
    }
}

/**
 * 布尔列
 */
class BooleanColumn implements IColumn {
    public readonly name: string;
    public readonly type: ColumnType = ColumnType.Boolean;
    private data: Uint8Array;
    private nullBitmap: Uint8Array;
    private _length: number;
    private _capacity: number;

    constructor(name: string, capacity: number) {
        this.name = name;
        this._capacity = capacity;
        this._length = 0;
        this.data = new Uint8Array(capacity);
        this.nullBitmap = new Uint8Array(Math.ceil(capacity / 8));
    }

    public get length(): number {
        return this._length;
    }

    public get(index: number): unknown {
        if (index < 0 || index >= this._length) {
            return undefined;
        }
        if (this.isNull(index)) {
            return null;
        }
        return this.data[index] === 1;
    }

    public set(index: number, value: unknown): void {
        if (index < 0 || index >= this._capacity) {
            throw new RangeError(`Index ${index} out of bounds`);
        }
        if (value === null || value === undefined) {
            this.setNull(index, true);
            this.data[index] = 0;
        } else {
            this.setNull(index, false);
            this.data[index] = value ? 1 : 0;
        }
        if (index >= this._length) {
            this._length = index + 1;
        }
    }

    public push(value: unknown): void {
        if (this._length >= this._capacity) {
            throw new Error("Column is full, call grow() first");
        }
        this.set(this._length, value);
    }

    public fill(value: unknown, start: number, end: number): void {
        for (let i = start; i < end; i++) {
            this.set(i, value);
        }
    }

    public getMemoryUsage(): number {
        return this.data.byteLength + this.nullBitmap.byteLength + 8;
    }

    public getNonNullCount(): number {
        let count = 0;
        for (let i = 0; i < this._length; i++) {
            if (!this.isNull(i)) {
                count++;
            }
        }
        return count;
    }

    public grow(newCapacity: number): void {
        if (newCapacity <= this._capacity) {
            return;
        }
        const newData = new Uint8Array(newCapacity);
        newData.set(this.data);
        this.data = newData;

        const newNullBitmap = new Uint8Array(Math.ceil(newCapacity / 8));
        newNullBitmap.set(this.nullBitmap);
        this.nullBitmap = newNullBitmap;

        this._capacity = newCapacity;
    }

    private isNull(index: number): boolean {
        const byteIndex = Math.floor(index / 8);
        const bitIndex = index % 8;
        return (this.nullBitmap[byteIndex]! & (1 << bitIndex)) !== 0;
    }

    private setNull(index: number, isNull: boolean): void {
        const byteIndex = Math.floor(index / 8);
        const bitIndex = index % 8;
        if (isNull) {
            this.nullBitmap[byteIndex]! |= 1 << bitIndex;
        } else {
            this.nullBitmap[byteIndex]! &= ~(1 << bitIndex);
        }
    }
}

/**
 * 列式存储
 * 使用平行数组存储数据，降低 GC 压力
 */
export class ColumnarStorage {
    private columns: Map<string, IColumn>;
    private columnOrder: string[];
    private capacity: number;
    private autoGrow: boolean;

    /**
     * 创建列式存储实例
     * @param options 配置选项
     */
    constructor(options: ColumnarStorageOptions) {
        this.columns = new Map();
        this.columnOrder = [];
        this.capacity = options.initialCapacity ?? 1024;
        this.autoGrow = options.autoGrow ?? true;

        for (const colDef of options.columns) {
            this.addColumn(colDef);
        }
    }

    /**
     * 添加列
     * @param definition 列定义
     */
    private addColumn(definition: ColumnDefinition): void {
        if (this.columns.has(definition.name)) {
            throw new Error(`Column "${definition.name}" already exists`);
        }

        const column = this.createColumn(definition);
        this.columns.set(definition.name, column);
        this.columnOrder.push(definition.name);
    }

    /**
     * 创建列实例
     */
    private createColumn(definition: ColumnDefinition): IColumn {
        const { name, type } = definition;

        switch (type) {
            case ColumnType.Int8:
                return new Int8Column(name, this.capacity);
            case ColumnType.Int16:
                return new Int16Column(name, this.capacity);
            case ColumnType.Int32:
                return new Int32Column(name, this.capacity);
            case ColumnType.Int64:
                return new Int64Column(name, this.capacity);
            case ColumnType.UInt8:
                return new UInt8Column(name, this.capacity);
            case ColumnType.UInt16:
                return new UInt16Column(name, this.capacity);
            case ColumnType.UInt32:
                return new UInt32Column(name, this.capacity);
            case ColumnType.UInt64:
                return new UInt64Column(name, this.capacity);
            case ColumnType.Float32:
                return new Float32Column(name, this.capacity);
            case ColumnType.Float64:
                return new Float64Column(name, this.capacity);
            case ColumnType.String:
                return new StringColumn(name, this.capacity);
            case ColumnType.Boolean:
                return new BooleanColumn(name, this.capacity);
            default:
                throw new Error(`Unsupported column type: ${type}`);
        }
    }

    /**
     * 获取行数
     */
    public get rowCount(): number {
        if (this.columnOrder.length === 0) {
            return 0;
        }
        const firstColumn = this.columns.get(this.columnOrder[0]!);
        return firstColumn ? firstColumn.length : 0;
    }

    /**
     * 获取列数
     */
    public get columnCount(): number {
        return this.columns.size;
    }

    /**
     * 获取列名称列表
     */
    public getColumnNames(): string[] {
        return [...this.columnOrder];
    }

    /**
     * 获取列类型
     * @param columnName 列名称
     */
    public getColumnType(columnName: string): ColumnType | undefined {
        const column = this.columns.get(columnName);
        return column?.type;
    }

    /**
     * 插入一行数据
     * @param row 行数据
     * @returns 插入的行索引
     */
    public insert(row: Record<string, unknown>): number {
        const rowIndex = this.rowCount;

        if (rowIndex >= this.capacity && this.autoGrow) {
            this.grow(this.capacity * 2);
        }

        for (const [colName, value] of Object.entries(row)) {
            const column = this.columns.get(colName);
            if (column) {
                column.push(value);
            }
        }

        for (const colName of this.columnOrder) {
            const column = this.columns.get(colName)!;
            if (column.length <= rowIndex) {
                column.push(null);
            }
        }

        return rowIndex;
    }

    /**
     * 批量插入数据
     * @param rows 行数据数组
     * @returns 插入的行索引范围
     */
    public insertMany(rows: Record<string, unknown>[]): {
        start: number;
        end: number;
    } {
        const start = this.rowCount;

        for (const row of rows) {
            this.insert(row);
        }

        return { start, end: this.rowCount };
    }

    /**
     * 获取单行数据
     * @param rowIndex 行索引
     */
    public getRow(rowIndex: number): Record<string, unknown> | null {
        if (rowIndex < 0 || rowIndex >= this.rowCount) {
            return null;
        }

        const row: Record<string, unknown> = {};
        for (const colName of this.columnOrder) {
            const column = this.columns.get(colName)!;
            row[colName] = column.get(rowIndex);
        }

        return row;
    }

    /**
     * 更新单行数据
     * @param rowIndex 行索引
     * @param row 新数据
     */
    public updateRow(rowIndex: number, row: Record<string, unknown>): boolean {
        if (rowIndex < 0 || rowIndex >= this.rowCount) {
            return false;
        }

        for (const [colName, value] of Object.entries(row)) {
            const column = this.columns.get(colName);
            if (column) {
                column.set(rowIndex, value);
            }
        }

        return true;
    }

    /**
     * 获取单列数据
     * @param columnName 列名称
     */
    public getColumn(columnName: string): unknown[] {
        const column = this.columns.get(columnName);
        if (!column) {
            return [];
        }

        const result: unknown[] = [];
        for (let i = 0; i < column.length; i++) {
            result.push(column.get(i));
        }

        return result;
    }

    /**
     * 获取类型化数组列（仅数值类型）
     * @param columnName 列名称
     */
    public getTypedColumn<T extends ArrayBufferView>(columnName: string): T | null {
        const column = this.columns.get(columnName);
        if (!column) {
            return null;
        }

        if (column instanceof TypedArrayColumn) {
            return column.getData() as T;
        }

        return null;
    }

    /**
     * 聚合计算
     * @param columnName 列名称
     * @param type 聚合类型
     */
    public aggregate(columnName: string, type: AggregationType): AggregationResult {
        const column = this.columns.get(columnName);
        if (!column) {
            return {
                type,
                field: columnName,
                value: null,
                recordCount: 0,
            };
        }

        const values: (number | bigint)[] = [];
        for (let i = 0; i < column.length; i++) {
            const val = column.get(i);
            if (val !== null && val !== undefined) {
                if (typeof val === "number" || typeof val === "bigint") {
                    values.push(val);
                }
            }
        }

        if (values.length === 0) {
            return {
                type,
                field: columnName,
                value: null,
                recordCount: 0,
            };
        }

        let value: number | bigint | null = null;

        switch (type) {
            case "sum": {
                if (typeof values[0] === "bigint") {
                    value = (values as bigint[]).reduce((a, b) => a + b, 0n);
                } else {
                    value = (values as number[]).reduce((a, b) => a + b, 0);
                }
                break;
            }
            case "avg": {
                if (typeof values[0] === "bigint") {
                    const sum = (values as bigint[]).reduce((a, b) => a + b, 0n);
                    value = Number(sum) / values.length;
                } else {
                    value = (values as number[]).reduce((a, b) => a + b, 0) / values.length;
                }
                break;
            }
            case "min": {
                value = values.reduce((a, b) => (a < b ? a : b));
                break;
            }
            case "max": {
                value = values.reduce((a, b) => (a > b ? a : b));
                break;
            }
            case "count": {
                value = values.length;
                break;
            }
            case "countDistinct": {
                const unique = new Set(values);
                value = unique.size;
                break;
            }
        }

        return {
            type,
            field: columnName,
            value,
            recordCount: values.length,
        };
    }

    /**
     * 范围扫描
     * @param columnName 列名称
     * @param condition 范围条件
     */
    public rangeScan(columnName: string, condition: RangeCondition): number[] {
        const column = this.columns.get(columnName);
        if (!column) {
            return [];
        }

        const result: number[] = [];

        for (let i = 0; i < column.length; i++) {
            const value = column.get(i);
            if (value === null || value === undefined) {
                continue;
            }

            if (this.matchRange(value, condition)) {
                result.push(i);
            }
        }

        return result;
    }

    /**
     * 匹配范围条件
     */
    private matchRange(value: unknown, condition: RangeCondition): boolean {
        if (typeof value !== "number" && typeof value !== "bigint" && typeof value !== "string") {
            return false;
        }

        const numValue = value as number | bigint | string;

        if (condition.gt !== undefined && condition.gt !== null) {
            if (!(numValue > (condition.gt as number | bigint | string))) {
                return false;
            }
        }
        if (condition.gte !== undefined && condition.gte !== null) {
            if (!(numValue >= (condition.gte as number | bigint | string))) {
                return false;
            }
        }
        if (condition.lt !== undefined && condition.lt !== null) {
            if (!(numValue < (condition.lt as number | bigint | string))) {
                return false;
            }
        }
        if (condition.lte !== undefined && condition.lte !== null) {
            if (!(numValue <= (condition.lte as number | bigint | string))) {
                return false;
            }
        }
        return true;
    }

    /**
     * 创建行迭代器
     * @param start 起始索引
     * @param end 结束索引
     */
    public *iterateRows(start: number = 0, end?: number): Generator<Record<string, unknown>, void, unknown> {
        const stop = end ?? this.rowCount;
        for (let i = start; i < stop; i++) {
            yield this.getRow(i)!;
        }
    }

    /**
     * 创建过滤迭代器
     * @param predicate 过滤条件函数
     */
    public *filter(predicate: (row: Record<string, unknown>, index: number) => boolean): Generator<Record<string, unknown>, void, unknown> {
        for (let i = 0; i < this.rowCount; i++) {
            const row = this.getRow(i)!;
            if (predicate(row, i)) {
                yield row;
            }
        }
    }

    /**
     * 扩容
     * @param newCapacity 新容量
     */
    public grow(newCapacity: number): void {
        if (newCapacity <= this.capacity) {
            return;
        }

        for (const column of this.columns.values()) {
            column.grow(newCapacity);
        }

        this.capacity = newCapacity;
    }

    /**
     * 清空数据
     */
    public clear(): void {
        for (const colName of this.columnOrder) {
            const column = this.columns.get(colName)!;
            column.fill(null, 0, column.length);
        }
    }

    /**
     * 获取统计信息
     */
    public getStats(): ColumnarStorageStats {
        const columnStats: ColumnStats[] = [];

        for (const colName of this.columnOrder) {
            const column = this.columns.get(colName)!;
            columnStats.push({
                name: colName,
                type: column.type,
                nonNullCount: column.getNonNullCount(),
                memoryUsage: column.getMemoryUsage(),
            });
        }

        const totalMemory = columnStats.reduce((sum, stat) => sum + stat.memoryUsage, 0);

        return {
            columnCount: this.columns.size,
            rowCount: this.rowCount,
            memoryUsage: totalMemory,
            columns: columnStats,
        };
    }

    /**
     * 获取容量
     */
    public getCapacity(): number {
        return this.capacity;
    }

    /**
     * 删除行（标记为 null，不实际删除）
     * @param rowIndex 行索引
     */
    public deleteRow(rowIndex: number): boolean {
        if (rowIndex < 0 || rowIndex >= this.rowCount) {
            return false;
        }

        for (const column of this.columns.values()) {
            column.set(rowIndex, null);
        }

        return true;
    }

    /**
     * 按条件删除行
     * @param predicate 条件函数
     * @returns 删除的行数
     */
    public deleteWhere(predicate: (row: Record<string, unknown>, index: number) => boolean): number {
        let count = 0;

        for (let i = 0; i < this.rowCount; i++) {
            const row = this.getRow(i)!;
            if (predicate(row, i)) {
                this.deleteRow(i);
                count++;
            }
        }

        return count;
    }
}
