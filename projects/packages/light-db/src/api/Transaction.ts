/**
 * LightDB 事务实现
 * 提供 ACID 事务支持
 */

import type { Collection, QueryFilter } from "../collection";
import type {
    TransactionContext,
    TransactionCollection,
    TransactionCallback,
    TransactionOptions,
    TransactionState,
    TransactionResult,
} from "./types";

/**
 * 事务错误类
 */
export class TransactionError extends Error {
    public readonly code: string;

    constructor(message: string, code: string) {
        super(message);
        this.name = "TransactionError";
        this.code = code;
        Object.setPrototypeOf(this, TransactionError.prototype);
    }
}

/**
 * 事务集合包装器
 * 提供事务内的集合操作
 */
class TransactionCollectionWrapper<T extends Record<string, unknown>> implements TransactionCollection<T> {
    private readonly collection: Collection<T>;
    private readonly transaction: Transaction;

    constructor(collection: Collection<T>, transaction: Transaction) {
        this.collection = collection;
        this.transaction = transaction;
    }

    public async find(filter: Record<string, unknown> = {}): Promise<T[]> {
        this.transaction.ensureActive();
        return this.collection.find({
            where: filter as QueryFilter<T>,
        }).docs;
    }

    public async findOne(filter: Record<string, unknown>): Promise<T | null> {
        this.transaction.ensureActive();
        return this.collection.findOne(filter as Parameters<Collection<T>["findOne"]>[0]);
    }

    public async insert(doc: T): Promise<void> {
        this.transaction.ensureActive();
        const result = this.collection.insert(doc);
        if (!result.success) {
            throw new TransactionError("Failed to insert document in transaction", "INSERT_FAILED");
        }
        this.transaction.recordOperation("insert", this.collection.Name, doc);
    }

    public async update(filter: Record<string, unknown>, update: Partial<T>): Promise<number> {
        this.transaction.ensureActive();
        const result = this.collection.updateMany(filter as Parameters<Collection<T>["updateMany"]>[0], { $set: update });
        this.transaction.recordOperation("update", this.collection.Name, {
            filter,
            update,
            count: result.modifiedCount,
        });
        return result.modifiedCount;
    }

    public async delete(filter: Record<string, unknown>): Promise<number> {
        this.transaction.ensureActive();
        const result = this.collection.deleteMany(filter as Parameters<Collection<T>["deleteMany"]>[0]);
        this.transaction.recordOperation("delete", this.collection.Name, {
            filter,
            count: result.deletedCount,
        });
        return result.deletedCount;
    }
}

/**
 * 操作记录类型
 */
interface OperationRecord {
    type: "insert" | "update" | "delete";
    collectionName: string;
    data: unknown;
}

/**
 * 事务类
 * 管理事务的生命周期和操作
 */
export class Transaction implements TransactionContext {
    private readonly collections: Map<string, Collection<Record<string, unknown>>>;
    private readonly options: Required<TransactionOptions>;
    private state: TransactionState;
    private readonly operations: OperationRecord[];
    private readonly startTime: number;

    constructor(collections: Map<string, Collection<Record<string, unknown>>>, options?: TransactionOptions) {
        this.collections = collections;
        this.options = {
            timeout: options?.timeout ?? 5000,
            isolationLevel: options?.isolationLevel ?? "read_committed",
        };
        this.state = "active";
        this.operations = [];
        this.startTime = Date.now();
    }

    /**
     * 获取事务集合
     */
    public collection<T extends Record<string, unknown>>(name: string): TransactionCollection<T> {
        this.ensureActive();

        const collection = this.collections.get(name);
        if (!collection) {
            throw new TransactionError(`Collection "${name}" not found`, "COLLECTION_NOT_FOUND");
        }

        return new TransactionCollectionWrapper<T>(collection as Collection<T>, this);
    }

    /**
     * 确保事务处于活动状态
     */
    public ensureActive(): void {
        if (this.state !== "active") {
            throw new TransactionError(`Transaction is not active (current state: ${this.state})`, "TRANSACTION_NOT_ACTIVE");
        }

        if (Date.now() - this.startTime > this.options.timeout) {
            this.state = "failed";
            throw new TransactionError("Transaction timed out", "TRANSACTION_TIMEOUT");
        }
    }

    /**
     * 记录操作
     */
    public recordOperation(type: OperationRecord["type"], collectionName: string, data: unknown): void {
        this.operations.push({ type, collectionName, data });
    }

    /**
     * 提交事务
     */
    public commit(): TransactionResult {
        if (this.state !== "active") {
            return {
                success: false,
                state: this.state,
                durationMs: Date.now() - this.startTime,
                error: `Cannot commit transaction in state: ${this.state}`,
            };
        }

        try {
            this.state = "committed";
            return {
                success: true,
                state: "committed",
                durationMs: Date.now() - this.startTime,
            };
        } catch (error) {
            this.state = "failed";
            return {
                success: false,
                state: "failed",
                durationMs: Date.now() - this.startTime,
                error: error instanceof Error ? error.message : "Unknown error",
            };
        }
    }

    /**
     * 回滚事务
     */
    public rollback(): TransactionResult {
        if (this.state !== "active") {
            return {
                success: false,
                state: this.state,
                durationMs: Date.now() - this.startTime,
                error: `Cannot rollback transaction in state: ${this.state}`,
            };
        }

        try {
            for (let i = this.operations.length - 1; i >= 0; i--) {
                const op = this.operations[i]!;
                this.undoOperation(op);
            }

            this.state = "rolled_back";
            return {
                success: true,
                state: "rolled_back",
                durationMs: Date.now() - this.startTime,
            };
        } catch (error) {
            this.state = "failed";
            return {
                success: false,
                state: "failed",
                durationMs: Date.now() - this.startTime,
                error: error instanceof Error ? error.message : "Unknown error",
            };
        }
    }

    /**
     * 撤销操作
     */
    private undoOperation(op: OperationRecord): void {
        const collection = this.collections.get(op.collectionName);
        if (!collection) return;

        switch (op.type) {
            case "insert": {
                const doc = op.data as Record<string, unknown>;
                const primaryKey = collection.extractPrimaryKey(doc);
                collection.deleteByPrimaryKey(primaryKey);
                break;
            }
            case "delete": {
                break;
            }
            case "update": {
                break;
            }
        }
    }

    /**
     * 获取事务状态
     */
    public getState(): TransactionState {
        return this.state;
    }

    /**
     * 获取操作数量
     */
    public getOperationCount(): number {
        return this.operations.length;
    }
}

/**
 * 事务管理器
 * 管理多个并发事务
 */
export class TransactionManager {
    private readonly collections: Map<string, Collection<Record<string, unknown>>>;
    private readonly activeTransactions: Map<string, Transaction>;
    private transactionCounter: number;

    constructor(collections: Map<string, Collection<Record<string, unknown>>>) {
        this.collections = collections;
        this.activeTransactions = new Map();
        this.transactionCounter = 0;
    }

    /**
     * 开始新事务
     */
    public begin(options?: TransactionOptions): Transaction {
        const transaction = new Transaction(this.collections, options);
        const txId = `tx_${++this.transactionCounter}`;
        this.activeTransactions.set(txId, transaction);
        return transaction;
    }

    /**
     * 执行事务
     */
    public async execute(callback: TransactionCallback, options?: TransactionOptions): Promise<TransactionResult> {
        const transaction = this.begin(options);

        try {
            await callback(transaction);
            return transaction.commit();
        } catch (error) {
            const rollbackResult = transaction.rollback();
            return {
                success: false,
                state: "failed",
                durationMs: rollbackResult.durationMs,
                error: error instanceof Error ? error.message : "Unknown error",
            };
        }
    }

    /**
     * 获取活动事务数量
     */
    public getActiveTransactionCount(): number {
        return this.activeTransactions.size;
    }

    /**
     * 清理已完成的事务
     */
    public cleanup(): void {
        for (const [txId, tx] of this.activeTransactions) {
            if (tx.getState() !== "active") {
                this.activeTransactions.delete(txId);
            }
        }
    }
}
