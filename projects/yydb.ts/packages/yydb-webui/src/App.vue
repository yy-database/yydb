<template>
    <main class="wrap">
        <header>
            <h1>YY WebUI</h1>
            <p class="sub">
                本地管理 · YY 二进制协议 / VOS · `YYDB`/`YYDS` 自称等效 · 无登录 / 无 ACL
            </p>
        </header>

        <aside class="warn">
            <strong>安全提示：</strong>
            参考实现 <code>yydb serve</code> 没有权限系统；连上 socket 即可读写。帧头
            <code>YYDB</code>/<code>YYDS</code> 只是后端自称，效果相同，前端不强求一致。
            请只连本机 loopback（默认
            <code>ws://127.0.0.1:7700/wire</code>），不要把 serve 暴露到公网。
        </aside>

        <section class="panel">
            <div class="row">
                <label style="flex: 1">
                    Endpoint
                    <input v-model="endpoint" placeholder="ws://127.0.0.1:7700/wire"/>
                </label>
                <button v-if="!client" @click="connect">Connect</button>
                <button v-else class="secondary" @click="disconnect">Disconnect</button>
            </div>
            <p v-if="status" class="mono">{{ status }}</p>
            <p v-if="error" class="error">{{ error }}</p>
        </section>

        <section v-if="client" class="grid two">
            <div class="panel">
                <h2>Info</h2>
                <button class="secondary" @click="refreshInfo">Refresh</button>
                <pre class="mono">{{ infoText || "(empty)" }}</pre>
            </div>

            <div class="panel">
                <h2>Schema (VOS)</h2>
                <label>
                    version
                    <input v-model.number="schemaVersion" type="number" min="1"/>
                </label>
                <label>
                    document
                    <textarea v-model="schemaDocument"/>
                </label>
                <div class="row">
                    <button class="secondary" @click="loadSchema">Load</button>
                    <button @click="saveSchema">Ensure</button>
                </div>
            </div>

            <div class="panel" style="grid-column: 1 / -1">
                <h2>Key / Value</h2>
                <div class="grid two">
                    <label>
                        key
                        <input v-model="kvKey"/>
                    </label>
                    <label>
                        value
                        <input v-model="kvValue"/>
                    </label>
                </div>
                <div class="row" style="margin-top: 0.75rem">
                    <button class="secondary" @click="kvGet">Get</button>
                    <button @click="kvPut">Put</button>
                </div>
                <pre class="mono">{{ kvResult }}</pre>
            </div>
        </section>
    </main>
</template>

<script setup lang="ts">
import {ref} from "vue";
import {Client} from "@yydb/yydb-client";

const endpoint = ref("ws://127.0.0.1:7700/wire");
const client = ref<Client | null>(null);
const status = ref("");
const error = ref("");
const infoText = ref("");
const schemaVersion = ref(1);
const schemaDocument = ref("table Project { @@id: uuid, title: utf8 }");
const kvKey = ref("demo");
const kvValue = ref("hello");
const kvResult = ref("");

async function connect() {
    error.value = "";
    try {
        const next = await Client.connect(endpoint.value);
        client.value = next;
        status.value = `connected · server ${await next.serverVersion()}`;
        await refreshInfo();
        await loadSchema();
    } catch (err) {
        error.value = String(err);
        client.value = null;
    }
}

function disconnect() {
    client.value?.close();
    client.value = null;
    status.value = "disconnected";
}

async function refreshInfo() {
    if (!client.value) return;
    infoText.value = await client.value.info();
}

async function loadSchema() {
    if (!client.value) return;
    const schema = await client.value.getSchema();
    if (!schema) {
        schemaDocument.value = "";
        return;
    }
    schemaVersion.value = schema.version;
    schemaDocument.value = schema.document;
}

async function saveSchema() {
    if (!client.value) return;
    error.value = "";
    try {
        await client.value.ensureSchema(schemaVersion.value, schemaDocument.value);
        status.value = "schema ensured";
        await refreshInfo();
    } catch (err) {
        error.value = String(err);
    }
}

async function kvGet() {
    if (!client.value) return;
    const value = await client.value.get(kvKey.value);
    kvResult.value = value
        ? new TextDecoder().decode(value)
        : "(missing)";
}

async function kvPut() {
    if (!client.value) return;
    await client.value.put(kvKey.value, kvValue.value);
    kvResult.value = "put ok";
    await refreshInfo();
}
</script>

<style scoped>
.wrap {
    max-width: 960px;
    margin: 0 auto;
    padding: 1.5rem 1rem 3rem;
    display: grid;
    gap: 1rem;
}

header h1 {
    margin: 0;
    font-size: 1.75rem;
}

.sub {
    margin: 0.25rem 0 0;
    color: var(--muted);
}

h2 {
    margin: 0 0 0.75rem;
    font-size: 1.1rem;
}
</style>
