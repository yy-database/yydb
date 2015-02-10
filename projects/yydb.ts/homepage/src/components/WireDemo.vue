<template>
    <section class="demo panel">
        <h2>本地帧编解码演示</h2>
        <p class="muted">
            使用 <code>@yydb/yydb-client</code> 在浏览器内编码/解码
            <code>YYDB</code>/<code>YYDS</code>（后端自称，效果相同）+ 版本
            <code>0000</code>（演进 <code>0001</code>）。前端不强求魔数与预期产品一致。
            不连接 live serve。
        </p>
        <label>
            request id
            <input v-model.number="requestId" type="number" min="1"/>
        </label>
        <label>
            Hello body (UTF-8，可空)
            <input v-model="bodyText"/>
        </label>
        <div class="row">
            <button type="button" @click="encode">Encode Hello</button>
            <button type="button" class="secondary" @click="roundtrip">
                Encode → Decode
            </button>
        </div>
        <pre class="mono">{{ hex }}</pre>
        <pre v-if="decoded" class="mono">{{ decoded }}</pre>
    </section>
</template>

<script setup lang="ts">
import {ref} from "vue";
import {MsgType, decodeFrame, encodeFrame} from "@yydb/yydb-client";

const requestId = ref(1);
const bodyText = ref("");
const hex = ref("");
const decoded = ref("");

function toHex(bytes: Uint8Array): string {
    return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join(" ");
}

function encode() {
    const body = new TextEncoder().encode(bodyText.value);
    const bytes = encodeFrame({
        msgType: MsgType.Hello,
        flags: 0,
        requestId: requestId.value >>> 0,
        body,
    });
    hex.value = toHex(bytes);
    decoded.value = "";
}

function roundtrip() {
    encode();
    const bytes = Uint8Array.from(
        hex.value.split(/\s+/).filter(Boolean).map((part) => parseInt(part, 16)),
    );
    const frame = decodeFrame(bytes);
    decoded.value = JSON.stringify(
        {
            msgType: frame.msgType,
            flags: frame.flags,
            requestId: frame.requestId,
            body: new TextDecoder().decode(frame.body),
        },
        null,
        2,
    );
}

encode();
</script>

<style scoped>
.panel {
    background: white;
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 1rem 1.1rem;
    margin-top: 1.25rem;
}

.muted {
    color: var(--muted);
}

label {
    display: grid;
    gap: 0.35rem;
    margin: 0.75rem 0;
    color: var(--muted);
    font-size: 0.9rem;
}

input {
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 0.45rem 0.6rem;
}

.row {
    display: flex;
    gap: 0.6rem;
    flex-wrap: wrap;
}

button {
    border: 0;
    border-radius: 6px;
    padding: 0.45rem 0.85rem;
    background: var(--accent);
    color: white;
    cursor: pointer;
}

button.secondary {
    background: #e8eef4;
    color: var(--ink);
}

.mono {
    font-family: ui-monospace, Consolas, monospace;
    white-space: pre-wrap;
    word-break: break-word;
    background: #101820;
    color: #e8eef4;
    padding: 0.75rem;
    border-radius: 8px;
}
</style>
