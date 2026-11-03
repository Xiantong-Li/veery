<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type Hit = { id: number; name: string; icon: string | null };
type SearchBody = { ready: boolean; hits: Hit[] };

const query = ref("");
const results = ref<Hit[]>([]);
const answered = ref("");
const indexing = ref(false);
const selected = ref(0);
const error = ref("");
const inputEl = ref<HTMLInputElement | null>(null);
const sheetEl = ref<HTMLElement | null>(null);
const menuEl = ref<HTMLElement | null>(null);
const menu = ref<{ id: number; x: number; y: number } | null>(null);
let request = 0;

const settled = computed(() => query.value.trim() !== "" && answered.value === query.value);
const drawerOpen = computed(
  () =>
    Boolean(error.value) ||
    indexing.value ||
    (query.value.trim() !== "" && (results.value.length > 0 || settled.value)),
);

let searchTimer = 0;

function scheduleRefresh() {
  window.clearTimeout(searchTimer);
  const text = query.value;
  menu.value = null;
  if (!text.trim()) {
    request += 1;
    results.value = [];
    selected.value = 0;
    answered.value = "";
    indexing.value = false;
    error.value = "";
    return;
  }
  if (text === answered.value && !error.value && !indexing.value) return;
  searchTimer = window.setTimeout(() => {
    void refresh();
  }, 40);
}

async function refresh() {
  const current = ++request;
  const text = query.value;
  if (!text.trim()) return;
  try {
    // 这里只能丢掉过期的返回值，已经发出的搜索仍会在后端算完。
    const body = await invoke<SearchBody>("search", { query: text });
    if (current !== request) return;
    indexing.value = !body.ready;
    error.value = "";
    if (!body.ready) return;
    results.value = body.hits;
    answered.value = text;
    if (selected.value >= results.value.length) {
      selected.value = 0;
    }
  } catch (cause) {
    if (current !== request) return;
    indexing.value = false;
    error.value = String(cause);
  }
}

async function act(command: "launch" | "launch_admin" | "reveal", id: number) {
  menu.value = null;
  error.value = "";
  try {
    await invoke(command, { id });
    query.value = "";
  } catch (cause) {
    error.value = String(cause);
  }
}

async function openSelected() {
  const hit = results.value[selected.value];
  if (!hit) return;
  await act("launch", hit.id);
}

function openMenu(index: number, event: MouseEvent) {
  const hit = results.value[index];
  if (!hit) return;
  selected.value = index;
  menu.value = { id: hit.id, x: event.clientX, y: event.clientY };
  void placeMenu();
}

function onPointerDown(event: PointerEvent) {
  const target = event.target;
  if (!(target instanceof Node)) return;
  if (menuEl.value?.contains(target)) return;
  if (sheetEl.value?.contains(target)) {
    if (!menu.value || event.button === 2) return;
    menu.value = null;
    return;
  }
  void invoke("hide_window");
}

async function placeMenu() {
  await nextTick();
  const node = menuEl.value;
  if (!node || !menu.value) return;
  const margin = 12;
  let x = menu.value.x;
  let y = menu.value.y;
  if (x + node.offsetWidth > window.innerWidth - margin) {
    x = window.innerWidth - node.offsetWidth - margin;
  }
  if (y + node.offsetHeight > window.innerHeight - margin) {
    y = menu.value.y - node.offsetHeight;
  }
  menu.value = {
    id: menu.value.id,
    x: Math.max(margin, x),
    y: Math.max(margin, y),
  };
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "ArrowDown") {
    event.preventDefault();
    if (results.value.length === 0) return;
    selected.value = (selected.value + 1) % results.value.length;
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    if (results.value.length === 0) return;
    selected.value =
      (selected.value + results.value.length - 1) % results.value.length;
  } else if (event.key === "Enter") {
    event.preventDefault();
    void openSelected();
  } else if (event.key === "Escape") {
    event.preventDefault();
    if (menu.value) {
      menu.value = null;
      return;
    }
    void invoke("hide_window");
  }
}

async function focusInput() {
  await nextTick();
  inputEl.value?.focus();
  inputEl.value?.select();
}

const stops: UnlistenFn[] = [];

onMounted(async () => {
  window.addEventListener("pointerdown", onPointerDown);
  try {
    stops.push(
      await listen("shown", async () => {
        query.value = "";
        error.value = "";
        selected.value = 0;
        results.value = [];
        answered.value = "";
        indexing.value = false;
        menu.value = null;
        await focusInput();
      }),
    );
    stops.push(
      await listen("hidden", async () => {
        query.value = "";
        error.value = "";
        selected.value = 0;
        results.value = [];
        answered.value = "";
        indexing.value = false;
        menu.value = null;
      }),
    );
    stops.push(
      await listen("catalog-ready", () => {
        indexing.value = false;
        results.value = [];
        answered.value = "";
        selected.value = 0;
        if (query.value.trim()) scheduleRefresh();
      }),
    );
  } catch (cause) {
    error.value = String(cause);
  }
  await focusInput();
});

onUnmounted(() => {
  window.clearTimeout(searchTimer);
  window.removeEventListener("pointerdown", onPointerDown);
  for (const stop of stops) stop();
});
</script>

<template>
  <main class="stage">
    <section ref="sheetEl" class="sheet">
      <input
        ref="inputEl"
        v-model="query"
        class="query"
        spellcheck="false"
        placeholder="搜索"
        @input="scheduleRefresh"
        @keydown="onKeydown"
      />
      <div class="drawer" :class="{ open: drawerOpen }">
        <div>
          <p v-if="error" class="note error">{{ error }}</p>
          <ul v-else-if="results.length" class="results">
        <li v-for="(item, index) in results" :key="item.id">
          <button
            type="button"
            :class="{ active: index === selected }"
            @mouseenter="selected = index"
            @mousedown.prevent
            @click="act('launch', item.id)"
            @contextmenu.prevent="openMenu(index, $event)"
          >
            <img v-if="item.icon" class="icon" :src="item.icon" alt="" />
            <span v-else class="icon placeholder"></span>
            <span class="name">{{ item.name }}</span>
          </button>
            </li>
          </ul>
          <p v-else-if="indexing" class="note">正在读取开始菜单</p>
          <p v-else-if="settled" class="note">没有匹配的程序</p>
        </div>
      </div>
    </section>
    <div
      v-if="menu"
      ref="menuEl"
      class="menu"
      :style="{ left: `${menu.x}px`, top: `${menu.y}px` }"
      @mousedown.prevent
      @contextmenu.prevent
    >
      <button type="button" @click="act('launch', menu.id)">打开</button>
      <button type="button" @click="act('launch_admin', menu.id)">以管理员身份运行</button>
      <button type="button" @click="act('reveal', menu.id)">打开所在文件夹</button>
    </div>
  </main>
</template>

<style>
html,
body,
#app {
  margin: 0;
  background: transparent;
  overflow: hidden;
  color-scheme: light;
}

body {
  font-family: "Segoe UI", "Microsoft YaHei", sans-serif;
  color: #c2c2c2;
  user-select: none;
  -webkit-font-smoothing: antialiased;
}

.stage {
  box-sizing: border-box;
  width: 960px;
  padding: 20px 20px 28px;
}

.sheet {
  background: #ffffff;
  border-radius: 16px;
  overflow: hidden;
  transform: translateZ(0);
  box-shadow:
    0 2px 6px rgba(0, 0, 0, 0.05),
    0 12px 32px rgba(0, 0, 0, 0.1);
}

.query {
  width: 100%;
  height: 84px;
  box-sizing: border-box;
  border: 0;
  outline: none;
  background: transparent;
  color: #c2c2c2;
  font: inherit;
  font-size: 32px;
  font-weight: 400;
  line-height: 84px;
  padding: 0 32px;
  caret-color: #c2c2c2;
  user-select: text;
}

.sheet:has(.drawer.open) .query {
  border-bottom: 1px solid #f0f0f0;
}

.drawer {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 0.28s cubic-bezier(0.32, 0.72, 0, 1);
}

.drawer.open {
  grid-template-rows: 1fr;
}

.drawer > div {
  min-height: 0;
  overflow: hidden;
}

.query::placeholder {
  color: #c2c2c2;
}

.results {
  list-style: none;
  margin: 0;
  padding: 6px 8px 8px;
}

.results button {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 12px;
  text-align: left;
  border: 0;
  background: transparent;
  color: #8a8a8a;
  font: inherit;
  font-size: 16px;
  border-radius: 12px;
  padding: 10px 18px;
  cursor: pointer;
}

.results button.active {
  color: #5c5c5c;
  background: #f4f4f4;
}

.name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.icon {
  width: 24px;
  height: 24px;
  flex: none;
  border-radius: 6px;
}

.placeholder {
  background: #f0f0f0;
}

.note {
  margin: 0;
  padding: 14px 32px 18px;
  color: #c2c2c2;
  font-size: 16px;
}

.error {
  color: #b42318;
}

.menu {
  position: fixed;
  z-index: 2;
  min-width: 188px;
  margin: 0;
  padding: 6px;
  border-radius: 8px;
  background: #ffffff;
  box-shadow:
    0 1px 2px rgba(0, 0, 0, 0.08),
    0 8px 24px rgba(0, 0, 0, 0.14);
}

.menu button {
  display: block;
  width: 100%;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: #1f1f1f;
  font: inherit;
  font-size: 14px;
  line-height: 1.2;
  text-align: left;
  padding: 8px 12px;
  cursor: pointer;
}

.menu button:hover {
  background: #f2f2f2;
}

@media (prefers-reduced-motion: reduce) {
  .drawer {
    transition: none;
  }
}
</style>
