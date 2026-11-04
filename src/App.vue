<script setup lang="ts">
import { computed, inject, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  applyTheme,
  copy,
  languageOf,
  prefsKey,
  themeOf,
  watchSystemTheme,
  type Language,
  type ThemeChoice,
} from "./prefs";

type Hit = { id: number; name: string; icon: string | null };
type SearchBody = { ready: boolean; hits: Hit[] };

const initial = inject(prefsKey, null);
const language = ref<Language>(languageOf(initial?.language));
const theme = ref<ThemeChoice>(themeOf(initial?.theme));
const t = computed(() => copy(language.value));
const hint = ref(initial?.hint ?? "");
const placeholder = computed(() => hint.value || t.value.search);

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
const hintMenuEl = ref<HTMLElement | null>(null);
const hintInputEl = ref<HTMLInputElement | null>(null);
const hintMenu = ref<{ x: number; y: number; editing: boolean } | null>(null);
const hintDraft = ref("");
let request = 0;

const settled = computed(() => query.value.trim() !== "" && answered.value === query.value);
const drawerOpen = computed(
  () =>
    Boolean(error.value) ||
    indexing.value ||
    (query.value.trim() !== "" && (results.value.length > 0 || settled.value)),
);

let searchTimer = 0;

watch(
  drawerOpen,
  (open) => {
    void invoke("set_launcher_open", { open }).catch(() => undefined);
  },
  { flush: "sync" },
);

function scheduleRefresh() {
  window.clearTimeout(searchTimer);
  const text = query.value;
  menu.value = null;
  hintMenu.value = null;
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
  hintMenu.value = null;
  menu.value = { id: hit.id, x: event.clientX, y: event.clientY };
  void placeMenu();
}

function openHintMenu(event: MouseEvent) {
  menu.value = null;
  hintDraft.value = hint.value;
  hintMenu.value = { x: event.clientX, y: event.clientY, editing: false };
  void placeHintMenu();
}

async function beginHintEdit() {
  if (!hintMenu.value) return;
  hintDraft.value = hint.value;
  hintMenu.value = { ...hintMenu.value, editing: true };
  await nextTick();
  hintInputEl.value?.focus();
  hintInputEl.value?.select();
  await placeHintMenu();
}

async function writeHint(next: string) {
  const previous = hint.value;
  hint.value = next.trim();
  hintMenu.value = null;
  try {
    hint.value = await invoke<string>("set_search_hint", { hint: hint.value });
  } catch (cause) {
    hint.value = previous;
    error.value = String(cause);
  }
  void focusInput();
}

function closeHintMenu() {
  hintMenu.value = null;
  void focusInput();
}

function onHintMenuMouseDown(event: MouseEvent) {
  if (hintMenu.value?.editing) return;
  event.preventDefault();
}

function onPointerDown(event: PointerEvent) {
  const target = event.target;
  if (!(target instanceof Node)) return;
  if (menuEl.value?.contains(target) || hintMenuEl.value?.contains(target)) return;
  if (sheetEl.value?.contains(target)) {
    if (event.button === 2) return;
    menu.value = null;
    hintMenu.value = null;
    return;
  }
  void invoke("hide_window");
}

async function placeAt(node: HTMLElement | null, point: { x: number; y: number } | null) {
  await nextTick();
  if (!node || !point) return null;
  const margin = 12;
  let x = point.x;
  let y = point.y;
  if (x + node.offsetWidth > window.innerWidth - margin) {
    x = window.innerWidth - node.offsetWidth - margin;
  }
  if (y + node.offsetHeight > window.innerHeight - margin) {
    y = point.y - node.offsetHeight;
  }
  return { x: Math.max(margin, x), y: Math.max(margin, y) };
}

async function placeMenu() {
  const placed = await placeAt(menuEl.value, menu.value);
  if (!placed || !menu.value) return;
  menu.value = { id: menu.value.id, x: placed.x, y: placed.y };
}

async function placeHintMenu() {
  const placed = await placeAt(hintMenuEl.value, hintMenu.value);
  if (!placed || !hintMenu.value) return;
  hintMenu.value = { ...hintMenu.value, x: placed.x, y: placed.y };
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
    if (menu.value || hintMenu.value) {
      menu.value = null;
      hintMenu.value = null;
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
let stopThemeWatch: (() => void) | null = null;

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
        hintMenu.value = null;
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
        hintMenu.value = null;
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
    stops.push(
      await listen<{ language: string; theme: string }>("appearance", (event) => {
        language.value = languageOf(event.payload.language);
        theme.value = themeOf(event.payload.theme);
        applyTheme(theme.value);
        document.documentElement.lang = language.value;
      }),
    );
  } catch (cause) {
    error.value = String(cause);
  }
  stopThemeWatch = watchSystemTheme(() => theme.value);
  await focusInput();
});

onUnmounted(() => {
  window.clearTimeout(searchTimer);
  window.removeEventListener("pointerdown", onPointerDown);
  for (const stop of stops) stop();
  stopThemeWatch?.();
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
        :placeholder="placeholder"
        @input="scheduleRefresh"
        @keydown="onKeydown"
        @contextmenu.prevent="openHintMenu"
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
          <p v-else-if="indexing" class="note">{{ t.indexing }}</p>
          <p v-else-if="settled" class="note">{{ t.empty }}</p>
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
      <button type="button" @click="act('launch', menu.id)">{{ t.open }}</button>
      <button type="button" @click="act('launch_admin', menu.id)">{{ t.admin }}</button>
      <button type="button" @click="act('reveal', menu.id)">{{ t.folder }}</button>
    </div>
    <div
      v-if="hintMenu"
      ref="hintMenuEl"
      class="menu"
      :style="{ left: `${hintMenu.x}px`, top: `${hintMenu.y}px` }"
      @mousedown="onHintMenuMouseDown"
      @contextmenu.prevent
    >
      <input
        v-if="hintMenu.editing"
        ref="hintInputEl"
        v-model="hintDraft"
        class="hint-input"
        maxlength="20"
        spellcheck="false"
        :placeholder="t.search"
        @keydown.enter.prevent="writeHint(hintDraft)"
        @keydown.escape.prevent="closeHintMenu"
      />
      <template v-else>
        <button type="button" @click="beginHintEdit">{{ t.customizeHint }}</button>
        <button v-if="hint" type="button" @click="writeHint('')">{{ t.resetHint }}</button>
      </template>
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
}

html[data-theme="dark"] {
  color-scheme: dark;
  --sheet: #2c2c2c;
  --query: #d0d0d0;
  --caret: #d0d0d0;
  --line: #3a3a3a;
  --result: #bdbdbd;
  --result-active: #f3f3f3;
  --result-bg: #3a3a3a;
  --note: #9a9a9a;
  --icon-bg: #3a3a3a;
  --menu-bg: #2c2c2c;
  --menu-text: #f3f3f3;
  --menu-hover: #3a3a3a;
  --error: #ffb4ab;
  --shadow:
    0 2px 6px rgba(0, 0, 0, 0.3),
    0 12px 32px rgba(0, 0, 0, 0.45);
}

html[data-theme="light"],
html:not([data-theme]) {
  color-scheme: light;
  --sheet: #ffffff;
  --query: #c2c2c2;
  --caret: #c2c2c2;
  --line: #f0f0f0;
  --result: #8a8a8a;
  --result-active: #5c5c5c;
  --result-bg: #f4f4f4;
  --note: #c2c2c2;
  --icon-bg: #f0f0f0;
  --menu-bg: #ffffff;
  --menu-text: #1f1f1f;
  --menu-hover: #f2f2f2;
  --error: #b42318;
  --shadow:
    0 2px 6px rgba(0, 0, 0, 0.05),
    0 12px 32px rgba(0, 0, 0, 0.1);
}

body {
  font-family: "Segoe UI", "Microsoft YaHei", sans-serif;
  color: var(--query);
  user-select: none;
  -webkit-font-smoothing: antialiased;
}

.stage {
  box-sizing: border-box;
  width: 960px;
  padding: 20px 20px 28px;
}

.sheet {
  background: var(--sheet);
  border-radius: 16px;
  overflow: hidden;
  transform: translateZ(0);
  box-shadow: var(--shadow);
}

.query {
  width: 100%;
  height: 84px;
  box-sizing: border-box;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--query);
  font: inherit;
  font-size: 32px;
  font-weight: 400;
  line-height: 84px;
  padding: 0 32px;
  caret-color: var(--caret);
  user-select: text;
}

.sheet:has(.drawer.open) .query {
  border-bottom: 1px solid var(--line);
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
  color: var(--query);
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
  color: var(--result);
  font: inherit;
  font-size: 16px;
  border-radius: 12px;
  padding: 10px 18px;
  cursor: pointer;
}

.results button.active {
  color: var(--result-active);
  background: var(--result-bg);
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
  background: var(--icon-bg);
}

.note {
  margin: 0;
  padding: 14px 32px 18px;
  color: var(--note);
  font-size: 16px;
}

.error {
  color: var(--error);
}

.menu {
  position: fixed;
  z-index: 2;
  min-width: 188px;
  margin: 0;
  padding: 6px;
  border-radius: 8px;
  background: var(--menu-bg);
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
  color: var(--menu-text);
  font: inherit;
  font-size: 14px;
  line-height: 1.2;
  text-align: left;
  padding: 8px 12px;
  cursor: pointer;
}

.menu button:hover {
  background: var(--menu-hover);
}

.hint-input {
  display: block;
  width: 100%;
  box-sizing: border-box;
  border: 0;
  border-radius: 4px;
  background: var(--menu-hover);
  color: var(--menu-text);
  font: inherit;
  font-size: 14px;
  line-height: 1.2;
  padding: 8px 12px;
  outline: none;
  user-select: text;
}

@media (prefers-reduced-motion: reduce) {
  .drawer {
    transition: none;
  }
}
</style>
