/**
 * 事务作者面
 *
 * 事务语义由 `core/engine` 维护，本层只提供异步调用形态与状态跟踪。
 */

import type { StorageEngine } from "../core/engine";
import { LightDbError, LightDbErrorCodes } from "../errors";

export type { TransactionState } from "../core/engine";

/**
 * 事务句柄
 *
 * 一个数据库同时只允许一个写事务；重复开始会得到 `yydb.unsupported`。
 */
export class Transaction {
    private readonly engine: StorageEngine;
    private active: boolean = false;

    constructor(engine: StorageEngine) {
        this.engine = engine;
    }

    /** 是否处于事务中 */
    public get isActive(): boolean {
        return this.active;
    }

    /** 开始事务 */
    public async begin(): Promise<void> {
        this.engine.begin();
        this.active = true;
    }

    /** 提交事务，失败时事务仍视为结束 */
    public async commit(): Promise<void> {
        if (!this.active) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "no active transaction", { operation: "commit" });
        }

        try {
            await this.engine.commit();
        } finally {
            this.active = false;
        }
    }

    /** 回滚事务 */
    public async rollback(): Promise<void> {
        if (!this.active) {
            throw new LightDbError(LightDbErrorCodes.unsupported, "no active transaction", { operation: "rollback" });
        }

        try {
            this.engine.rollback();
        } finally {
            this.active = false;
        }
    }
}
