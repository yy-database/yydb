/**
 * LightDB B+ 树实现
 * 用于主键索引，支持 O(log n) 的插入、查找、删除和范围查询
 */

import type { Comparator, IndexKey } from "./types";
import { IndexOperationError } from "./errors";

/**
 * 默认 B+ 树阶数（每个节点最多存储的键数量）
 */
const DEFAULT_ORDER = 64;

/**
 * B+ 树叶子节点
 */
class BPlusTreeLeafNode<K, V> {
    public keys: K[] = [];
    public values: V[] = [];
    public next: BPlusTreeLeafNode<K, V> | null = null;
    public prev: BPlusTreeLeafNode<K, V> | null = null;

    public isLeaf(): true {
        return true;
    }
}

/**
 * B+ 树内部节点
 */
class BPlusTreeInternalNode<K, V> {
    public keys: K[] = [];
    public children: Array<BPlusTreeLeafNode<K, V> | BPlusTreeInternalNode<K, V>> = [];

    public isLeaf(): false {
        return false;
    }
}

type BPlusTreeNode<K, V> = BPlusTreeLeafNode<K, V> | BPlusTreeInternalNode<K, V>;

/**
 * 默认比较器
 * 支持字符串和数值类型的比较
 */
function defaultComparator<K extends IndexKey>(a: K, b: K): number {
    if (typeof a === "number" && typeof b === "number") {
        return a - b;
    }
    const strA = String(a);
    const strB = String(b);
    if (strA < strB) return -1;
    if (strA > strB) return 1;
    return 0;
}

/**
 * B+ 树实现
 * 提供高效的键值存储和范围查询能力
 */
export class BPlusTree<K extends IndexKey, V> {
    private root: BPlusTreeNode<K, V>;
    private readonly order: number;
    private readonly comparator: Comparator<K>;
    private size: number = 0;
    private firstLeaf: BPlusTreeLeafNode<K, V> | null = null;
    private lastLeaf: BPlusTreeLeafNode<K, V> | null = null;

    /**
     * 创建 B+ 树实例
     * @param order B+ 树阶数，默认 64
     * @param comparator 键比较器
     */
    constructor(order: number = DEFAULT_ORDER, comparator?: Comparator<K>) {
        if (order < 3) {
            throw new IndexOperationError("B+ tree order must be at least 3", "constructor");
        }
        this.order = order;
        this.comparator = comparator ?? defaultComparator;
        this.root = new BPlusTreeLeafNode<K, V>();
        this.firstLeaf = this.root as BPlusTreeLeafNode<K, V>;
        this.lastLeaf = this.root as BPlusTreeLeafNode<K, V>;
    }

    /**
     * 获取树中键值对数量
     */
    public get Size(): number {
        return this.size;
    }

    /**
     * 检查树是否为空
     */
    public isEmpty(): boolean {
        return this.size === 0;
    }

    /**
     * 插入键值对
     * @param key 键
     * @param value 值
     * @returns 如果键已存在，返回旧值；否则返回 undefined
     */
    public insert(key: K, value: V): V | undefined {
        const existingValue = this.find(key);
        if (existingValue !== undefined) {
            this.updateValue(key, value);
            return existingValue;
        }

        const leaf = this.findLeafNode(key);
        this.insertIntoLeaf(leaf, key, value);
        this.size++;

        if (leaf.keys.length > this.order) {
            this.splitLeaf(leaf);
        }

        return undefined;
    }

    /**
     * 查找键对应的值
     * @param key 键
     * @returns 值，如果不存在返回 undefined
     */
    public find(key: K): V | undefined {
        const leaf = this.findLeafNode(key);
        const index = this.binarySearch(leaf.keys, key);

        if (index >= 0 && index < leaf.keys.length) {
            const cmp = this.comparator(leaf.keys[index]!, key);
            if (cmp === 0) {
                return leaf.values[index];
            }
        }

        for (let i = 0; i < leaf.keys.length; i++) {
            if (this.comparator(leaf.keys[i]!, key) === 0) {
                return leaf.values[i];
            }
        }

        return undefined;
    }

    /**
     * 检查键是否存在
     * @param key 键
     */
    public has(key: K): boolean {
        return this.find(key) !== undefined;
    }

    /**
     * 删除键值对
     * @param key 键
     * @returns 被删除的值，如果不存在返回 undefined
     */
    public delete(key: K): V | undefined {
        const leaf = this.findLeafNode(key);
        const index = this.findKeyIndex(leaf.keys, key);

        if (index === -1) {
            return undefined;
        }

        const value = leaf.values[index];
        leaf.keys.splice(index, 1);
        leaf.values.splice(index, 1);
        this.size--;

        if (leaf.keys.length === 0 && leaf !== this.root) {
            this.removeLeafNode(leaf);
        }

        return value;
    }

    /**
     * 更新键对应的值
     * @param key 键
     * @param value 新值
     * @returns 是否更新成功
     */
    public update(key: K, value: V): boolean {
        const leaf = this.findLeafNode(key);
        const index = this.findKeyIndex(leaf.keys, key);

        if (index === -1) {
            return false;
        }

        leaf.values[index] = value;
        return true;
    }

    /**
     * 范围查询
     * @param start 起始键（包含）
     * @param end 结束键（包含）
     * @param includeStart 是否包含起始键
     * @param includeEnd 是否包含结束键
     * @returns 键值对数组
     */
    public range(start?: K, end?: K, includeStart: boolean = true, includeEnd: boolean = true): Array<{ key: K; value: V }> {
        const result: Array<{ key: K; value: V }> = [];

        if (this.size === 0) {
            return result;
        }

        let startLeaf: BPlusTreeLeafNode<K, V>;
        let startIndex: number;

        if (start !== undefined) {
            startLeaf = this.findLeafNode(start);
            startIndex = this.findFirstGreaterOrEqual(startLeaf.keys, start);
        } else {
            startLeaf = this.firstLeaf!;
            startIndex = 0;
        }

        let currentLeaf: BPlusTreeLeafNode<K, V> | null = startLeaf;
        let currentIndex = startIndex;

        while (currentLeaf !== null) {
            for (let i = currentIndex; i < currentLeaf.keys.length; i++) {
                const key = currentLeaf.keys[i]!;

                if (start !== undefined) {
                    const cmp = this.comparator(key, start);
                    if (cmp < 0 || (cmp === 0 && !includeStart)) {
                        continue;
                    }
                }

                if (end !== undefined) {
                    const cmp = this.comparator(key, end);
                    if (cmp > 0 || (cmp === 0 && !includeEnd)) {
                        return result;
                    }
                }

                result.push({ key, value: currentLeaf.values[i]! });
            }

            currentLeaf = currentLeaf.next;
            currentIndex = 0;
        }

        return result;
    }

    /**
     * 获取所有键（有序）
     */
    public keys(): K[] {
        const result: K[] = [];
        let current = this.firstLeaf;

        while (current !== null) {
            result.push(...current.keys);
            current = current.next;
        }

        return result;
    }

    /**
     * 获取所有值（按键顺序）
     */
    public values(): V[] {
        const result: V[] = [];
        let current = this.firstLeaf;

        while (current !== null) {
            result.push(...current.values);
            current = current.next;
        }

        return result;
    }

    /**
     * 获取所有键值对（有序）
     */
    public entries(): Array<{ key: K; value: V }> {
        const result: Array<{ key: K; value: V }> = [];
        let current = this.firstLeaf;

        while (current !== null) {
            for (let i = 0; i < current.keys.length; i++) {
                result.push({ key: current.keys[i]!, value: current.values[i]! });
            }
            current = current.next;
        }

        return result;
    }

    /**
     * 清空树
     */
    public clear(): void {
        this.root = new BPlusTreeLeafNode<K, V>();
        this.firstLeaf = this.root as BPlusTreeLeafNode<K, V>;
        this.lastLeaf = this.root as BPlusTreeLeafNode<K, V>;
        this.size = 0;
    }

    /**
     * 获取最小键
     */
    public minKey(): K | undefined {
        if (this.size === 0) {
            return undefined;
        }
        return this.firstLeaf!.keys[0];
    }

    /**
     * 获取最大键
     */
    public maxKey(): K | undefined {
        if (this.size === 0) {
            return undefined;
        }
        return this.lastLeaf!.keys[this.lastLeaf!.keys.length - 1];
    }

    /**
     * 获取第一个键值对
     */
    public first(): { key: K; value: V } | undefined {
        if (this.size === 0) {
            return undefined;
        }
        return {
            key: this.firstLeaf!.keys[0]!,
            value: this.firstLeaf!.values[0]!,
        };
    }

    /**
     * 获取最后一个键值对
     */
    public last(): { key: K; value: V } | undefined {
        if (this.size === 0) {
            return undefined;
        }
        const lastIndex = this.lastLeaf!.keys.length - 1;
        return {
            key: this.lastLeaf!.keys[lastIndex]!,
            value: this.lastLeaf!.values[lastIndex]!,
        };
    }

    /**
     * 遍历树
     * @param callback 回调函数
     */
    public forEach(callback: (key: K, value: V) => void): void {
        let current = this.firstLeaf;

        while (current !== null) {
            for (let i = 0; i < current.keys.length; i++) {
                callback(current.keys[i]!, current.values[i]!);
            }
            current = current.next;
        }
    }

    /**
     * 创建迭代器
     */
    public *[Symbol.iterator](): Generator<{ key: K; value: V }> {
        let current = this.firstLeaf;

        while (current !== null) {
            for (let i = 0; i < current.keys.length; i++) {
                yield { key: current.keys[i]!, value: current.values[i]! };
            }
            current = current.next;
        }
    }

    /**
     * 查找键所在的叶子节点
     */
    private findLeafNode(key: K): BPlusTreeLeafNode<K, V> {
        let node = this.root;

        while (!node.isLeaf()) {
            const internalNode = node as BPlusTreeInternalNode<K, V>;
            let i = 0;

            while (i < internalNode.keys.length && this.comparator(key, internalNode.keys[i]!) >= 0) {
                i++;
            }

            node = internalNode.children[i]!;
        }

        return node as BPlusTreeLeafNode<K, V>;
    }

    /**
     * 在叶子节点中插入键值对
     */
    private insertIntoLeaf(leaf: BPlusTreeLeafNode<K, V>, key: K, value: V): void {
        let i = 0;
        while (i < leaf.keys.length && this.comparator(key, leaf.keys[i]!) > 0) {
            i++;
        }

        leaf.keys.splice(i, 0, key);
        leaf.values.splice(i, 0, value);
    }

    /**
     * 分裂叶子节点
     */
    private splitLeaf(leaf: BPlusTreeLeafNode<K, V>): void {
        const mid = Math.floor(leaf.keys.length / 2);
        const newLeaf = new BPlusTreeLeafNode<K, V>();

        newLeaf.keys = leaf.keys.splice(mid);
        newLeaf.values = leaf.values.splice(mid);

        newLeaf.next = leaf.next;
        newLeaf.prev = leaf;
        if (leaf.next) {
            leaf.next.prev = newLeaf;
        }
        leaf.next = newLeaf;

        if (leaf === this.lastLeaf) {
            this.lastLeaf = newLeaf;
        }

        const parent = this.findOrCreateParent(leaf);
        const insertIndex = parent.children.indexOf(leaf) + 1;
        const promoteKey = newLeaf.keys[0]!;

        parent.keys.splice(insertIndex - 1, 0, promoteKey);
        parent.children.splice(insertIndex, 0, newLeaf);

        if (parent.keys.length >= this.order) {
            this.splitInternal(parent);
        }
    }

    /**
     * 分裂内部节点
     */
    private splitInternal(node: BPlusTreeInternalNode<K, V>): void {
        const mid = Math.floor(node.keys.length / 2);
        const newNode = new BPlusTreeInternalNode<K, V>();

        const promoteKey = node.keys[mid]!;

        newNode.keys = node.keys.splice(mid + 1);
        node.keys.pop();
        newNode.children = node.children.splice(mid + 1);

        if (node === this.root) {
            const newRoot = new BPlusTreeInternalNode<K, V>();
            newRoot.keys.push(promoteKey);
            newRoot.children.push(node, newNode);
            this.root = newRoot;
        } else {
            const parent = this.findOrCreateParent(node);
            const insertIndex = parent.children.indexOf(node) + 1;

            parent.keys.splice(insertIndex - 1, 0, promoteKey);
            parent.children.splice(insertIndex, 0, newNode);

            if (parent.keys.length >= this.order) {
                this.splitInternal(parent);
            }
        }
    }

    /**
     * 查找或创建父节点
     */
    private findOrCreateParent(node: BPlusTreeNode<K, V>): BPlusTreeInternalNode<K, V> {
        if (node === this.root) {
            const newRoot = new BPlusTreeInternalNode<K, V>();
            newRoot.children.push(this.root);
            this.root = newRoot;
            return newRoot;
        }

        return this.findParent(this.root, node) as BPlusTreeInternalNode<K, V>;
    }

    /**
     * 查找父节点
     */
    private findParent(current: BPlusTreeNode<K, V>, target: BPlusTreeNode<K, V>): BPlusTreeInternalNode<K, V> | null {
        if (current.isLeaf()) {
            return null;
        }

        const internalNode = current as BPlusTreeInternalNode<K, V>;

        for (const child of internalNode.children) {
            if (child === target) {
                return internalNode;
            }

            if (!child.isLeaf()) {
                const found = this.findParent(child, target);
                if (found) {
                    return found;
                }
            }
        }

        return null;
    }

    /**
     * 移除空叶子节点
     */
    private removeLeafNode(leaf: BPlusTreeLeafNode<K, V>): void {
        if (leaf.prev) {
            leaf.prev.next = leaf.next;
        } else {
            this.firstLeaf = leaf.next;
        }

        if (leaf.next) {
            leaf.next.prev = leaf.prev;
        } else {
            this.lastLeaf = leaf.prev;
        }

        if (leaf === this.root) {
            this.root = new BPlusTreeLeafNode<K, V>();
            this.firstLeaf = this.root as BPlusTreeLeafNode<K, V>;
            this.lastLeaf = this.root as BPlusTreeLeafNode<K, V>;
            return;
        }

        const parent = this.findParent(this.root, leaf);
        if (parent) {
            const index = parent.children.indexOf(leaf);
            if (index !== -1) {
                parent.children.splice(index, 1);
                if (index > 0) {
                    parent.keys.splice(index - 1, 1);
                } else if (parent.keys.length > 0) {
                    parent.keys.splice(0, 1);
                }
            }
        }
    }

    /**
     * 二分查找
     */
    private binarySearch(keys: K[], key: K): number {
        let left = 0;
        let right = keys.length - 1;

        while (left <= right) {
            const mid = Math.floor((left + right) / 2);
            const cmp = this.comparator(keys[mid]!, key);

            if (cmp === 0) {
                return mid;
            } else if (cmp < 0) {
                left = mid + 1;
            } else {
                right = mid - 1;
            }
        }

        return left;
    }

    /**
     * 查找键在数组中的索引
     */
    private findKeyIndex(keys: K[], key: K): number {
        for (let i = 0; i < keys.length; i++) {
            if (this.comparator(keys[i]!, key) === 0) {
                return i;
            }
        }
        return -1;
    }

    /**
     * 查找第一个大于等于目标键的索引
     */
    private findFirstGreaterOrEqual(keys: K[], key: K): number {
        for (let i = 0; i < keys.length; i++) {
            if (this.comparator(keys[i]!, key) >= 0) {
                return i;
            }
        }
        return keys.length;
    }

    /**
     * 更新已存在键的值
     */
    private updateValue(key: K, value: V): void {
        const leaf = this.findLeafNode(key);
        const index = this.findKeyIndex(leaf.keys, key);
        if (index !== -1) {
            leaf.values[index] = value;
        }
    }
}
