/**
 * 跨标签页通知
 *
 * 多标签页各自持有内存视图，提交、schema 变更和关闭需要广播，
 * 其他标签页据此丢弃陈旧视图。
 */

/** 数据库事件类型 */
export type DatabaseEventType = "commit" | "schema" | "close";

/** 数据库事件 */
export interface DatabaseEvent {
    /** 事件类型 */
    type: DatabaseEventType;
    /** 数据库名 */
    name: string;
    /** 提交后的 LSN，仅 `commit` 事件携带 */
    lsn?: string;
}

/** BroadcastChannel 结构化类型，避免引入 DOM lib */
interface BroadcastChannelLike {
    postMessage(message: unknown): void;
    close(): void;
    onmessage: ((event: { data: unknown }) => void) | null;
}

/**
 * 数据库事件广播器
 */
export class DatabaseBroadcaster {
    private readonly channel: BroadcastChannelLike | null;

    constructor(name: string) {
        this.channel = createChannel(`yydb:${name}`);
    }

    /**
     * 发送事件
     * @param event 事件
     */
    public notify(event: DatabaseEvent): void {
        this.channel?.postMessage(event);
    }

    /** 关闭广播 */
    public close(): void {
        this.channel?.close();
    }
}

/**
 * 监听数据库事件
 * @param name 数据库名
 * @param handler 处理函数
 * @returns 取消监听
 */
export function listenDatabaseEvents(name: string, handler: (event: DatabaseEvent) => void): () => void {
    const channel = createChannel(`yydb:${name}`);
    if (!channel) {
        return () => undefined;
    }

    channel.onmessage = (event) => {
        const data = event.data as DatabaseEvent | null;
        if (data && data.name === name) {
            handler(data);
        }
    };

    return () => channel.close();
}

/**
 * 创建广播通道
 * @param channelName 通道名
 */
function createChannel(channelName: string): BroadcastChannelLike | null {
    const constructor = (globalThis as unknown as { BroadcastChannel?: new (name: string) => BroadcastChannelLike })
        .BroadcastChannel;
    return constructor ? new constructor(channelName) : null;
}
