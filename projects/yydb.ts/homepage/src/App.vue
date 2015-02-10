<template>
    <div class="layout">
        <aside class="sidebar">
            <div class="brand">YY</div>
            <nav class="nav">
                <button
                    v-for="doc in docs"
                    :key="doc.id"
                    :class="{ active: current === doc.id }"
                    @click="current = doc.id"
                >
                    {{ doc.title }}
                </button>
            </nav>
        </aside>
        <main class="content">
            <article class="markdown" v-html="html"/>
            <WireDemo v-if="current === 'wire'"/>
        </main>
    </div>
</template>

<script setup lang="ts">
import {computed, ref} from "vue";
import {marked} from "marked";
import {docs} from "./docs";
import WireDemo from "./components/WireDemo.vue";

const current = ref(docs[0]!.id);
const html = computed(() => {
    const doc = docs.find((item) => item.id === current.value) ?? docs[0]!;
    return marked.parse(doc.body, {async: false}) as string;
});
</script>
