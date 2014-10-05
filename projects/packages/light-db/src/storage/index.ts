/**
 * storage 层导出
 *
 * `StorageAdapter` 是唯一平台边界；OPFS 为默认后端，内存后端用于测试与一次性会话。
 */

export { STORAGE_FILES } from './StorageAdapter';
export type { StorageAdapter, StorageAdapterFactory, StorageAdapterKind, StorageFile, StorageHandle, StorageOpenMode } from './StorageAdapter';
export { MemoryStorageAdapter, MemoryStorageBackend } from './memory/MemoryStorageAdapter';
export { OpfsStorageAdapter, OpfsStorageAdapterFactory, isOpfsAvailable } from './opfs/OpfsStorageAdapter';
export { IndexedDbStorageAdapter, IndexedDbStorageAdapterFactory, isIndexedDbAvailable } from './indexeddb/IndexedDbStorageAdapter';
