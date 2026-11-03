<script setup lang="ts">
import { nextTick, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Hit = { id: number; name: string; icon: string | null };

const WINDOW_WIDTH = 960;

const query = ref("");
const results = ref<Hit[]>([]);
const selected = ref(0);
const error = ref("");
const inputEl = ref<HTMLInputElement | null>(null);
const stageEl = ref<HTMLElement | null>(null);
const menuEl = ref<HTMLElement | null>(null);
const menu = ref<{ id: number; x: number; y: number } | null>(null);
let fitted = 0;
let request = 0;

async function fit() {
  await nextTick();
  const node = stageEl.value;
  if (!node) return;
  let height = Math.ceil(node.getBoundingClientRect().height);
  if (menuEl.value) {
    height = Math.max(height, Math.ceil(menuEl.value.getBoundingClientRect().bottom + 16));
  }
  if (height === fitted) return;
  fitted = height;
  try {
    await getCurrentWindow().setSize(new LogicalSize(WINDOW_WIDTH, height));
  } catch {
    // 浏览器预览里没有窗口接口
  }
}

async function refresh() {
  const current = ++request;
  const text = query.value;
  menu.value = null;
  if (!text.trim()) {
    results.value = [];
    selected.value = 0;
    await fit();
    return;
  }
  const hits = await invoke<Hit[]>("search", { query: text });
  if (current !== request) return;
  results.value = hits;
  if (selected.value >= results.value.length) {
    selected.value = 0;
  }
  await fit();
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
  await fit();
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
  await fit();
}

function closeMenu(event: PointerEvent) {
  if (!menu.value || event.button === 2) return;
  const target = event.target;
  if (target instanceof Node && menuEl.value?.contains(target)) return;
  menu.value = null;
  void fit();
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
      void fit();
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

onMounted(async () => {
  window.addEventListener("pointerdown", closeMenu);
  await listen("shown", async () => {
    query.value = "";
    error.value = "";
    selected.value = 0;
    results.value = [];
    menu.value = null;
    await focusInput();
    await fit();
  });
  await listen("hidden", async () => {
    query.value = "";
    error.value = "";
    selected.value = 0;
    results.value = [];
    menu.value = null;
    await fit();
  });
  await focusInput();
  await fit();
});
</script>

<template>
  <main ref="stageEl" class="stage">
    <section class="sheet">
      <input
        ref="inputEl"
        v-model="query"
        class="query"
        spellcheck="false"
        placeholder="搜索"
        @input="refresh"
        @keydown="onKeydown"
      />
      <p v-if="error" class="note error">{{ error }}</p>
      <ul v-else-if="query.trim() && results.length" class="results">
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
      <p v-else-if="query.trim()" class="note">没有匹配的程序</p>
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

.sheet:has(.results, .note) .query {
  border-bottom: 1px solid #f0f0f0;
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
</style>
