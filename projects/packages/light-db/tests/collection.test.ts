/**
 * 集合层单元测试
 * 测试 Collection、QueryBuilder、QueryParser
 */

import {
    describe,
    assertEqual,
    assertTrue,
    assertFalse,
    assertNotNull,
    assertNull,
    assertThrows,
    assertNoThrow,
    assertLength,
    TestSuite,
} from "./test-utils";
import { Collection, CollectionError } from "../src/collection/Collection";
import { QueryBuilder, QueryExecutor } from "../src/collection/QueryBuilder";
import { QueryParser } from "../src/collection/QueryParser";
import type {
    CollectionOptions,
    InsertOptions,
    UpdateOptions,
    DeleteOptions,
    QueryFilter,
} from "../src/collection/types";

/**
 * 测试文档类型
 */
interface TestUser {
    id: number;
    name: string;
    age: number;
    email: string;
    active: boolean;
    tags: string[];
    profile?: {
        bio: string;
        location: string;
    };
}

/**
 * 创建测试集合
 */
function createTestCollection(): Collection<TestUser> {
    const options: CollectionOptions<TestUser> = {
        name: "users",
        primaryKey: "id",
        indexes: [
            { name: "name_idx", fieldName: "name" },
            { name: "email_idx", fieldName: "email", unique: true },
        ],
    };

    return new Collection<TestUser>(options);
}

/**
 * Collection 测试套件
 */
function createCollectionTests(): TestSuite {
    const suite = describe("Collection Tests");

    suite.test("创建集合", () => {
        const collection = createTestCollection();

        assertEqual(collection.Name, "users");
        assertEqual(collection.PrimaryKeyField, "id");
        assertEqual(collection.Size, 0);
    });

    suite.test("插入单条文档", () => {
        const collection = createTestCollection();

        const doc: TestUser = {
            id: 1,
            name: "Alice",
            age: 25,
            email: "alice@example.com",
            active: true,
            tags: ["developer", "typescript"],
        };

        const result = collection.insert(doc);

        assertTrue(result.success);
        assertEqual(result.insertedCount, 1);
        assertEqual(result.insertedDocs.length, 1);
        assertEqual(collection.Size, 1);
    });

    suite.test("插入重复主键", () => {
        const collection = createTestCollection();

        const doc: TestUser = {
            id: 1,
            name: "Alice",
            age: 25,
            email: "alice@example.com",
            active: true,
            tags: [],
        };

        collection.insert(doc);
        const result = collection.insert(doc);

        assertFalse(result.success);
        assertEqual(result.insertedCount, 0);
    });

    suite.test("插入重复主键抛出异常", () => {
        const collection = createTestCollection();

        const doc: TestUser = {
            id: 1,
            name: "Alice",
            age: 25,
            email: "alice@example.com",
            active: true,
            tags: [],
        };

        collection.insert(doc);

        assertThrows(
            () => collection.insert(doc, { throwOnDuplicate: true }),
            CollectionError,
        );
    });

    suite.test("批量插入文档", () => {
        const collection = createTestCollection();

        const docs: TestUser[] = [
            { id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] },
            { id: 2, name: "Bob", age: 30, email: "bob@example.com", active: false, tags: [] },
            { id: 3, name: "Charlie", age: 28, email: "charlie@example.com", active: true, tags: [] },
        ];

        const result = collection.insertMany(docs);

        assertTrue(result.success);
        assertEqual(result.insertedCount, 3);
        assertEqual(result.failedCount, 0);
        assertEqual(collection.Size, 3);
    });

    suite.test("批量插入部分失败", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        const docs: TestUser[] = [
            { id: 1, name: "Alice2", age: 26, email: "alice2@example.com", active: true, tags: [] },
            { id: 2, name: "Bob", age: 30, email: "bob@example.com", active: false, tags: [] },
        ];

        const result = collection.insertMany(docs);

        assertFalse(result.success);
        assertEqual(result.insertedCount, 1);
        assertEqual(result.failedCount, 1);
        assertNotNull(result.errors);
    });

    suite.test("查找所有文档", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });
        collection.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com", active: false, tags: [] });

        const result = collection.find();

        assertEqual(result.docs.length, 2);
    });

    suite.test("查找单条文档", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });
        collection.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com", active: false, tags: [] });

        const result = collection.findOne({ name: "Alice" });

        assertNotNull(result);
        assertEqual(result!.id, 1);
        assertEqual(result!.name, "Alice");
    });

    suite.test("查找不存在的文档", () => {
        const collection = createTestCollection();

        const result = collection.findOne({ name: "NonExistent" });

        assertNull(result);
    });

    suite.test("通过主键查找", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        const result = collection.findByPrimaryKey(1);

        assertNotNull(result);
        assertEqual(result!.name, "Alice");
    });

    suite.test("通过二级索引查找", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });
        collection.insert({ id: 2, name: "Alice", age: 30, email: "alice2@example.com", active: false, tags: [] });

        const results = collection.findBySecondaryIndex("name_idx", "Alice");

        assertEqual(results.length, 2);
    });

    suite.test("更新单条文档", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        const result = collection.updateOne({ id: 1 }, { $set: { age: 26 } });

        assertTrue(result.success);
        assertEqual(result.matchedCount, 1);
        assertEqual(result.modifiedCount, 1);

        const updated = collection.findByPrimaryKey(1);
        assertEqual(updated!.age, 26);
    });

    suite.test("更新多条文档", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });
        collection.insert({ id: 2, name: "Bob", age: 25, email: "bob@example.com", active: false, tags: [] });
        collection.insert({ id: 3, name: "Charlie", age: 30, email: "charlie@example.com", active: true, tags: [] });

        const result = collection.updateMany({ age: 25 }, { $set: { age: 26 } });

        assertTrue(result.success);
        assertEqual(result.matchedCount, 2);
        assertEqual(result.modifiedCount, 2);
    });

    suite.test("更新不存在的文档", () => {
        const collection = createTestCollection();

        const result = collection.updateOne({ id: 999 }, { $set: { name: "Test" } });

        assertFalse(result.success);
        assertEqual(result.matchedCount, 0);
    });

    suite.test("Upsert 插入新文档", () => {
        const collection = createTestCollection();

        const result = collection.updateOne(
            { id: 1 },
            { $set: { name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] } },
            { upsert: true },
        );

        assertTrue(result.success);
        assertTrue(result.upserted);
        assertEqual(result.upsertedId, 1);
    });

    suite.test("使用 $set 更新", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        collection.updateOne({ id: 1 }, { $set: { name: "Alice Smith", age: 26 } });

        const doc = collection.findByPrimaryKey(1);
        assertEqual(doc!.name, "Alice Smith");
        assertEqual(doc!.age, 26);
        assertEqual(doc!.email, "alice@example.com");
    });

    suite.test("使用 $unset 删除字段", () => {
        const collection = createTestCollection();

        collection.insert({
            id: 1,
            name: "Alice",
            age: 25,
            email: "alice@example.com",
            active: true,
            tags: [],
            profile: { bio: "Developer", location: "NYC" },
        });

        collection.updateOne({ id: 1 }, { $unset: { profile: true } });

        const doc = collection.findByPrimaryKey(1);
        assertNull(doc!.profile);
    });

    suite.test("使用 $inc 增加数值", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        collection.updateOne({ id: 1 }, { $inc: { age: 5 } });

        const doc = collection.findByPrimaryKey(1);
        assertEqual(doc!.age, 30);
    });

    suite.test("使用 $push 添加数组元素", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: ["developer"] });

        collection.updateOne({ id: 1 }, { $push: { tags: "typescript" } });

        const doc = collection.findByPrimaryKey(1);
        assertEqual(doc!.tags.length, 2);
        assertTrue(doc!.tags.includes("typescript"));
    });

    suite.test("使用 $pull 删除数组元素", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: ["developer", "typescript"] });

        collection.updateOne({ id: 1 }, { $pull: { tags: "developer" } });

        const doc = collection.findByPrimaryKey(1);
        assertEqual(doc!.tags.length, 1);
        assertFalse(doc!.tags.includes("developer"));
    });

    suite.test("删除单条文档", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });
        collection.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com", active: false, tags: [] });

        const result = collection.deleteOne({ name: "Alice" });

        assertTrue(result.success);
        assertEqual(result.deletedCount, 1);
        assertEqual(collection.Size, 1);
    });

    suite.test("删除多条文档", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });
        collection.insert({ id: 2, name: "Bob", age: 25, email: "bob@example.com", active: false, tags: [] });
        collection.insert({ id: 3, name: "Charlie", age: 30, email: "charlie@example.com", active: true, tags: [] });

        const result = collection.deleteMany({ age: 25 });

        assertTrue(result.success);
        assertEqual(result.deletedCount, 2);
        assertEqual(collection.Size, 1);
    });

    suite.test("删除不存在的文档", () => {
        const collection = createTestCollection();

        const result = collection.deleteOne({ id: 999 });

        assertFalse(result.success);
        assertEqual(result.deletedCount, 0);
    });

    suite.test("通过主键删除", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        const result = collection.deleteByPrimaryKey(1);

        assertTrue(result);
        assertEqual(collection.Size, 0);
    });

    suite.test("统计文档数量", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });
        collection.insert({ id: 2, name: "Bob", age: 25, email: "bob@example.com", active: false, tags: [] });
        collection.insert({ id: 3, name: "Charlie", age: 30, email: "charlie@example.com", active: true, tags: [] });

        assertEqual(collection.count(), 3);
        assertEqual(collection.count({ age: 25 }), 2);
    });

    suite.test("检查文档是否存在", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        assertTrue(collection.exists({ name: "Alice" }));
        assertFalse(collection.exists({ name: "Bob" }));
    });

    suite.test("清空集合", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });
        collection.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com", active: false, tags: [] });

        collection.clear();

        assertEqual(collection.Size, 0);
    });

    suite.test("获取统计信息", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        const stats = collection.getStats();

        assertEqual(stats.name, "users");
        assertEqual(stats.documentCount, 1);
        assertGreater(stats.indexCount, 0);
    });

    suite.test("创建二级索引", () => {
        const collection = createTestCollection();

        collection.createIndex({ name: "age_idx", fieldName: "age" });

        const indexNames = collection.getIndexNames();
        assertTrue(indexNames.includes("age_idx"));
    });

    suite.test("删除二级索引", () => {
        const collection = createTestCollection();

        collection.createIndex({ name: "age_idx", fieldName: "age" });
        const result = collection.dropIndex("age_idx");

        assertTrue(result);
        assertFalse(collection.getIndexNames().includes("age_idx"));
    });

    suite.test("事件监听 - insert", () => {
        const collection = createTestCollection();

        let eventFired = false;
        let eventData: any = null;

        collection.on("insert", (event, data) => {
            eventFired = true;
            eventData = data;
        });

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        assertTrue(eventFired);
        assertNotNull(eventData);
        assertNotNull(eventData.doc);
    });

    suite.test("事件监听 - update", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        let eventFired = false;
        collection.on("update", () => {
            eventFired = true;
        });

        collection.updateOne({ id: 1 }, { $set: { age: 26 } });

        assertTrue(eventFired);
    });

    suite.test("事件监听 - delete", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        let eventFired = false;
        collection.on("delete", () => {
            eventFired = true;
        });

        collection.deleteOne({ id: 1 });

        assertTrue(eventFired);
    });

    suite.test("移除事件监听器", () => {
        const collection = createTestCollection();

        let callCount = 0;
        const listener = () => {
            callCount++;
        };

        collection.on("insert", listener);
        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        collection.off("insert", listener);
        collection.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com", active: false, tags: [] });

        assertEqual(callCount, 1);
    });

    suite.test("验证集合一致性", () => {
        const collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: [] });

        const result = collection.validate();

        assertTrue(result.valid);
    });

    return suite;
}

/**
 * QueryBuilder 测试套件
 */
function createQueryBuilderTests(): TestSuite {
    const suite = describe("QueryBuilder Tests");

    let collection: Collection<TestUser>;

    suite.beforeEach(() => {
        collection = createTestCollection();

        collection.insert({ id: 1, name: "Alice", age: 25, email: "alice@example.com", active: true, tags: ["developer"] });
        collection.insert({ id: 2, name: "Bob", age: 30, email: "bob@example.com", active: false, tags: ["designer"] });
        collection.insert({ id: 3, name: "Charlie", age: 25, email: "charlie@example.com", active: true, tags: ["developer", "manager"] });
        collection.insert({ id: 4, name: "David", age: 35, email: "david@example.com", active: true, tags: [] });
    });

    suite.test("创建查询构建器", () => {
        const query = collection.query();

        assertNotNull(query);
    });

    suite.test("查询所有文档", () => {
        const result = collection.query().toArray();

        assertEqual(result.docs.length, 4);
    });

    suite.test("where 条件查询", () => {
        const result = collection.query()
            .where({ name: "Alice" })
            .toArray();

        assertEqual(result.docs.length, 1);
        assertEqual(result.docs[0]!.name, "Alice");
    });

    suite.test("eq 等于条件", () => {
        const result = collection.query()
            .eq("name", "Alice")
            .toArray();

        assertEqual(result.docs.length, 1);
    });

    suite.test("ne 不等于条件", () => {
        const result = collection.query()
            .ne("name", "Alice")
            .toArray();

        assertEqual(result.docs.length, 3);
    });

    suite.test("gt 大于条件", () => {
        const result = collection.query()
            .gt("age", 25)
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("gte 大于等于条件", () => {
        const result = collection.query()
            .gte("age", 25)
            .toArray();

        assertEqual(result.docs.length, 4);
    });

    suite.test("lt 小于条件", () => {
        const result = collection.query()
            .lt("age", 30)
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("lte 小于等于条件", () => {
        const result = collection.query()
            .lte("age", 30)
            .toArray();

        assertEqual(result.docs.length, 3);
    });

    suite.test("in 包含于条件", () => {
        const result = collection.query()
            .in("name", ["Alice", "Bob"])
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("nin 不包含于条件", () => {
        const result = collection.query()
            .nin("name", ["Alice", "Bob"])
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("fieldExists 存在性检查", () => {
        const result = collection.query()
            .fieldExists("profile", false)
            .toArray();

        assertEqual(result.docs.length, 4);
    });

    suite.test("regex 正则匹配", () => {
        const result = collection.query()
            .regex("email", /example\.com$/)
            .toArray();

        assertEqual(result.docs.length, 4);
    });

    suite.test("and 逻辑与", () => {
        const result = collection.query()
            .and({ age: 25 }, { active: true })
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("or 逻辑或", () => {
        const result = collection.query()
            .or({ name: "Alice" }, { name: "Bob" })
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("not 逻辑非", () => {
        const result = collection.query()
            .not({ name: "Alice" })
            .toArray();

        assertEqual(result.docs.length, 3);
    });

    suite.test("sortBy 排序", () => {
        const result = collection.query()
            .sortBy("age", "asc")
            .toArray();

        assertEqual(result.docs[0]!.age, 25);
        assertEqual(result.docs[3]!.age, 35);
    });

    suite.test("asc 升序排序", () => {
        const result = collection.query()
            .asc("age")
            .toArray();

        assertEqual(result.docs[0]!.age, 25);
    });

    suite.test("desc 降序排序", () => {
        const result = collection.query()
            .desc("age")
            .toArray();

        assertEqual(result.docs[0]!.age, 35);
    });

    suite.test("skip 跳过记录", () => {
        const result = collection.query()
            .sortBy("id", "asc")
            .skip(2)
            .toArray();

        assertEqual(result.docs.length, 2);
        assertEqual(result.docs[0]!.id, 3);
    });

    suite.test("limit 限制记录数", () => {
        const result = collection.query()
            .limit(2)
            .toArray();

        assertEqual(result.docs.length, 2);
    });

    suite.test("select 字段投影", () => {
        const result = collection.query()
            .select("id", "name")
            .toArray();

        assertEqual(result.docs.length, 4);
        assertNotNull(result.docs[0]!.id);
        assertNotNull(result.docs[0]!.name);
    });

    suite.test("first 获取第一条", () => {
        const result = collection.query()
            .where({ age: 25 })
            .first();

        assertNotNull(result);
        assertEqual(result!.age, 25);
    });

    suite.test("last 获取最后一条", () => {
        const result = collection.query()
            .sortBy("id", "asc")
            .last();

        assertNotNull(result);
        assertEqual(result!.id, 4);
    });

    suite.test("count 统计数量", () => {
        const count = collection.query()
            .where({ age: 25 })
            .count();

        assertEqual(count, 2);
    });

    suite.test("hasMatch 检查是否有匹配", () => {
        assertTrue(collection.query().where({ name: "Alice" }).hasMatch());
        assertFalse(collection.query().where({ name: "NonExistent" }).hasMatch());
    });

    suite.test("组合查询", () => {
        const result = collection.query()
            .where({ active: true })
            .gt("age", 25)
            .sortBy("age", "asc")
            .limit(2)
            .toArray();

        assertEqual(result.docs.length, 1);
        assertEqual(result.docs[0]!.name, "David");
    });

    suite.test("getFilter 获取查询条件", () => {
        const query = collection.query()
            .where({ name: "Alice" })
            .gt("age", 20);

        const filter = query.getFilter();

        assertNotNull(filter);
        assertEqual(filter.name, "Alice");
    });

    suite.test("getSort 获取排序条件", () => {
        const query = collection.query()
            .sortBy("age", "asc");

        const sort = query.getSort();

        assertNotNull(sort);
        assertEqual(sort.age, "asc");
    });

    suite.test("reset 重置查询", () => {
        const query = collection.query()
            .where({ name: "Alice" })
            .limit(10);

        query.reset();

        assertEqual(query.getSkip(), 0);
        assertEqual(query.getLimit(), 0);
    });

    suite.test("clone 克隆查询", () => {
        const query1 = collection.query()
            .where({ name: "Alice" })
            .limit(10);

        const query2 = query1.clone();

        assertEqual(query1.getLimit(), query2.getLimit());
    });

    return suite;
}

/**
 * QueryParser 测试套件
 */
function createQueryParserTests(): TestSuite {
    const suite = describe("QueryParser Tests");

    suite.test("匹配空条件", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, {}));
    });

    suite.test("匹配精确值", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { name: "Alice" }));
        assertFalse(QueryParser.matches(doc, { name: "Bob" }));
    });

    suite.test("匹配 $eq", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { name: { $eq: "Alice" } }));
        assertFalse(QueryParser.matches(doc, { name: { $eq: "Bob" } }));
    });

    suite.test("匹配 $ne", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { name: { $ne: "Bob" } }));
        assertFalse(QueryParser.matches(doc, { name: { $ne: "Alice" } }));
    });

    suite.test("匹配 $gt", () => {
        const doc = { id: 1, age: 25 };

        assertTrue(QueryParser.matches(doc, { age: { $gt: 20 } }));
        assertFalse(QueryParser.matches(doc, { age: { $gt: 25 } }));
    });

    suite.test("匹配 $gte", () => {
        const doc = { id: 1, age: 25 };

        assertTrue(QueryParser.matches(doc, { age: { $gte: 25 } }));
        assertFalse(QueryParser.matches(doc, { age: { $gte: 26 } }));
    });

    suite.test("匹配 $lt", () => {
        const doc = { id: 1, age: 25 };

        assertTrue(QueryParser.matches(doc, { age: { $lt: 30 } }));
        assertFalse(QueryParser.matches(doc, { age: { $lt: 25 } }));
    });

    suite.test("匹配 $lte", () => {
        const doc = { id: 1, age: 25 };

        assertTrue(QueryParser.matches(doc, { age: { $lte: 25 } }));
        assertFalse(QueryParser.matches(doc, { age: { $lte: 24 } }));
    });

    suite.test("匹配 $in", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { name: { $in: ["Alice", "Bob"] } }));
        assertFalse(QueryParser.matches(doc, { name: { $in: ["Charlie", "David"] } }));
    });

    suite.test("匹配 $nin", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { name: { $nin: ["Charlie", "David"] } }));
        assertFalse(QueryParser.matches(doc, { name: { $nin: ["Alice", "Bob"] } }));
    });

    suite.test("匹配 $exists", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { name: { $exists: true } }));
        assertTrue(QueryParser.matches(doc, { email: { $exists: false } }));
        assertFalse(QueryParser.matches(doc, { email: { $exists: true } }));
    });

    suite.test("匹配 $regex", () => {
        const doc = { id: 1, email: "alice@example.com" };

        assertTrue(QueryParser.matches(doc, { email: { $regex: /example\.com$/ } }));
        assertFalse(QueryParser.matches(doc, { email: { $regex: /test\.com$/ } }));
    });

    suite.test("匹配 $and", () => {
        const doc = { id: 1, name: "Alice", age: 25 };

        assertTrue(QueryParser.matches(doc, { $and: [{ name: "Alice" }, { age: 25 }] }));
        assertFalse(QueryParser.matches(doc, { $and: [{ name: "Alice" }, { age: 30 }] }));
    });

    suite.test("匹配 $or", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { $or: [{ name: "Alice" }, { name: "Bob" }] }));
        assertFalse(QueryParser.matches(doc, { $or: [{ name: "Charlie" }, { name: "David" }] }));
    });

    suite.test("匹配 $not", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { $not: { name: "Bob" } }));
        assertFalse(QueryParser.matches(doc, { $not: { name: "Alice" } }));
    });

    suite.test("匹配 $nor", () => {
        const doc = { id: 1, name: "Alice" };

        assertTrue(QueryParser.matches(doc, { $nor: [{ name: "Bob" }, { name: "Charlie" }] }));
        assertFalse(QueryParser.matches(doc, { $nor: [{ name: "Alice" }, { name: "Bob" }] }));
    });

    suite.test("匹配嵌套字段", () => {
        const doc = {
            id: 1,
            profile: {
                bio: "Developer",
                location: "NYC",
            },
        };

        assertTrue(QueryParser.matches(doc, { "profile.location": "NYC" }));
        assertFalse(QueryParser.matches(doc, { "profile.location": "LA" }));
    });

    suite.test("分析查询 - 可使用索引", () => {
        const indexFields = ["id", "name"];

        const result = QueryParser.analyzeQuery({ name: "Alice" }, indexFields);

        assertTrue(result.canUseIndex);
        assertEqual(result.indexField, "name");
        assertTrue(result.isExactMatch);
    });

    suite.test("分析查询 - 不可使用索引", () => {
        const indexFields = ["id"];

        const result = QueryParser.analyzeQuery({ name: "Alice" }, indexFields);

        assertFalse(result.canUseIndex);
    });

    suite.test("分析查询 - $in 操作符", () => {
        const indexFields = ["id", "name"];

        const result = QueryParser.analyzeQuery({ name: { $in: ["Alice", "Bob"] } }, indexFields);

        assertTrue(result.canUseIndex);
        assertFalse(result.isExactMatch);
    });

    suite.test("提取查询字段", () => {
        const filter = {
            name: "Alice",
            age: { $gt: 20 },
            $or: [{ email: "test@example.com" }],
        };

        const fields = QueryParser.extractFields(filter);

        assertTrue(fields.includes("name"));
        assertTrue(fields.includes("age"));
        assertTrue(fields.includes("email"));
    });

    return suite;
}

/**
 * 导出所有集合层测试
 */
export function getCollectionTests(): TestSuite[] {
    return [
        createCollectionTests(),
        createQueryBuilderTests(),
        createQueryParserTests(),
    ];
}
