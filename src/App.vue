<script setup lang="ts">
import { computed, inject, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import SettingsPanel from "./SettingsPanel.vue";
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
type FileHit = { id: number; name: string; icon: string | null };
type Row =
  | { kind: "app"; id: number; name: string; icon: string | null }
  | { kind: "file"; id: number; name: string; icon: string | null };
type SearchBody = { ready: boolean; hits: Hit[] };
type FileSearchBody = { ready: boolean; hits: FileHit[] };

const initial = inject(prefsKey, null);
const language = ref<Language>(languageOf(initial?.language));
const theme = ref<ThemeChoice>(themeOf(initial?.theme));
const t = computed(() => copy(language.value));
const hint = ref(initial?.hint ?? "");
const placeholder = computed(() => hint.value || t.value.search);

const query = ref("");
const results = ref<Hit[]>([]);
const fileHits = ref<FileHit[]>([]);
const fileSearchOn = ref(Boolean(initial?.fileSearch));
const fileTypeCount = ref(initial?.fileTypes.length ?? 0);
const fileSearching = ref(false);
const fileIndexing = ref(false);
const answered = ref("");
const indexing = ref(false);
const searching = ref(false);
const selected = ref(0);
const error = ref("");
const settingsOpen = ref(false);
const settingsMounted = ref(false);
const resumeSettings = ref(false);
const recording = ref(false);
const update = ref<{ latest: string; url: string } | null>(null);
let suppressSettingsFocusClose = false;
const inputEl = ref<HTMLInputElement | null>(null);
const sheetEl = ref<HTMLElement | null>(null);
const menuEl = ref<HTMLElement | null>(null);
const menu = ref<{ kind: "app" | "file"; id: number; x: number; y: number } | null>(null);
const panelEl = ref<InstanceType<typeof SettingsPanel> | null>(null);
let request = 0;
let fileRequest = 0;

const settled = computed(() => query.value.trim() !== "" && answered.value === query.value);
const fileWanted = computed(
  () =>
    fileSearchOn.value &&
    fileTypeCount.value > 0 &&
    !settingsOpen.value &&
    query.value.trim().length >= 2,
);
const rows = computed<Row[]>(() => [
  ...results.value.map((hit) => ({ kind: "app" as const, ...hit })),
  ...fileHits.value.map((hit) => ({ kind: "file" as const, ...hit })),
]);
const appCount = computed(() => results.value.length);
const searchOpen = computed(
  () =>
    !settingsOpen.value &&
    (Boolean(error.value) ||
      indexing.value ||
      searching.value ||
      fileIndexing.value ||
      fileSearching.value ||
      (query.value.trim() !== "" && (results.value.length > 0 || fileHits.value.length > 0 || settled.value))),
);
const note = computed<{ text: string; error: boolean } | null>(() => {
  if (error.value) return { text: error.value, error: true };
  if (results.value.length > 0 || fileHits.value.length > 0) return null;
  if (indexing.value) return { text: t.value.indexing, error: false };
  if (fileIndexing.value) return { text: t.value.indexingFiles, error: false };
  if (searching.value) return { text: t.value.searching, error: false };
  if (fileSearching.value) return { text: t.value.searchingFiles, error: false };
  if (settled.value) {
    return { text: fileWanted.value ? t.value.emptyAll : t.value.empty, error: false };
  }
  return null;
});
const drawerOpen = computed(() => settingsOpen.value || searchOpen.value);

const DRAWER_MS = 280;
let searchTimer = 0;
let fileTimer = 0;
let frameTimer = 0;
let settingsWarmTimer = 0;
let resultsClearTimer = 0;

const frameMode = computed(() => {
  if (settingsOpen.value) return "settings";
  if (searchOpen.value || menu.value) return "open";
  return "idle";
});

watch(
  frameMode,
  (mode) => {
    window.clearTimeout(frameTimer);
    if (mode === "idle") {
      // 等抽屉收起动画走完再缩窗口，和展开时先长高再打开对齐。
      frameTimer = window.setTimeout(() => {
        void invoke("set_launcher_frame", { mode: "idle" }).catch(() => undefined);
      }, DRAWER_MS);
      return;
    }
    void invoke("set_launcher_frame", { mode }).catch(() => undefined);
  },
  { flush: "sync" },
);

function resetFiles() {
  window.clearTimeout(fileTimer);
  fileRequest += 1;
  fileSearching.value = false;
  fileIndexing.value = false;
  fileHits.value = [];
}

function clearSearch(immediate = false) {
  window.clearTimeout(searchTimer);
  window.clearTimeout(resultsClearTimer);
  request += 1;
  resetFiles();
  query.value = "";
  answered.value = "";
  indexing.value = false;
  searching.value = false;
  error.value = "";
  menu.value = null;
  if (immediate || results.value.length === 0) {
    results.value = [];
    selected.value = 0;
    return;
  }
  resultsClearTimer = window.setTimeout(() => {
    results.value = [];
    selected.value = 0;
  }, DRAWER_MS);
}

function closeSettings(options: { keepResume?: boolean } = {}) {
  if (recording.value) {
    void panelEl.value?.stopRecording();
  }
  if (!options.keepResume) {
    resumeSettings.value = false;
  }
  settingsOpen.value = false;
  recording.value = false;
  // 设置面板保持挂载，下次展开不再重新创建整棵树。
}

function warmSettingsPanel() {
  if (settingsMounted.value) return;
  settingsMounted.value = true;
}

function openSettingsPanel() {
  clearSearch(true);
  warmSettingsPanel();
  settingsOpen.value = true;
  recording.value = false;
}

function toggleSettings() {
  menu.value = null;
  if (settingsOpen.value) {
    // 关设置后再聚焦输入框时，忽略这次 focus，避免又 toggle 开回去。
    suppressSettingsFocusClose = true;
    closeSettings();
    void focusInput().finally(() => {
      window.setTimeout(() => {
        suppressSettingsFocusClose = false;
      }, 0);
    });
    return;
  }
  resumeSettings.value = false;
  openSettingsPanel();
}

function onInputFocus() {
  if (suppressSettingsFocusClose) return;
  if (settingsOpen.value) {
    closeSettings();
  }
}

function onQueryInput(event: Event) {
  const target = event.target;
  if (!(target instanceof HTMLInputElement)) return;
  if (settingsOpen.value) closeSettings();
  query.value = target.value;
  scheduleRefresh();
}

function scheduleRefresh() {
  window.clearTimeout(searchTimer);
  const text = query.value;
  menu.value = null;
  if (settingsOpen.value) {
    resetFiles();
    return;
  }
  if (!text.trim()) {
    resetFiles();
    request += 1;
    answered.value = "";
    indexing.value = false;
    searching.value = false;
    error.value = "";
    window.clearTimeout(resultsClearTimer);
    if (results.value.length === 0) {
      selected.value = 0;
      return;
    }
    // 先收起抽屉，动画后再清结果，避免内容瞬间消失。
    resultsClearTimer = window.setTimeout(() => {
      results.value = [];
      selected.value = 0;
    }, DRAWER_MS);
    return;
  }
  window.clearTimeout(resultsClearTimer);
  scheduleFileRefresh();
  if (text === answered.value && !error.value && !indexing.value && !searching.value) return;
  // 还没有可展示的结果时先进入搜索中，抽屉展开并给出反馈，避免干等像卡住。
  if (results.value.length === 0) {
    searching.value = true;
  }
  searchTimer = window.setTimeout(() => {
    void refresh();
  }, 40);
}

function scheduleFileRefresh() {
  window.clearTimeout(fileTimer);
  if (!fileWanted.value) {
    resetFiles();
    return;
  }
  // 文件范围比程序大，防抖久一点，避免每个按键都扫一遍。
  if (fileHits.value.length === 0) {
    fileSearching.value = true;
  }
  fileTimer = window.setTimeout(() => {
    void refreshFiles();
  }, 300);
}

async function refreshFiles() {
  const current = ++fileRequest;
  const text = query.value.trim();
  if (!fileWanted.value || text.length < 2) {
    fileSearching.value = false;
    return;
  }
  try {
    const body = await invoke<FileSearchBody>("search_files", { query: text });
    if (current !== fileRequest || !fileWanted.value) return;
    if (!body.ready) {
      fileIndexing.value = true;
      fileSearching.value = false;
      fileHits.value = [];
      return;
    }
    fileIndexing.value = false;
    fileSearching.value = false;
    fileHits.value = body.hits;
    if (selected.value >= results.value.length + fileHits.value.length) {
      selected.value = 0;
    }
  } catch {
    if (current !== fileRequest) return;
    fileIndexing.value = false;
    fileSearching.value = false;
    fileHits.value = [];
  }
}

async function refresh() {
  const current = ++request;
  const text = query.value;
  if (!text.trim() || settingsOpen.value) {
    searching.value = false;
    return;
  }
  if (results.value.length === 0) {
    searching.value = true;
  }
  try {
    const body = await invoke<SearchBody>("search", { query: text });
    if (current !== request || settingsOpen.value) return;
    indexing.value = !body.ready;
    error.value = "";
    if (!body.ready) {
      // 索引未好时继续显示读取提示，不算搜索结束。
      searching.value = false;
      return;
    }
    results.value = body.hits;
    answered.value = text;
    searching.value = false;
    if (selected.value >= results.value.length + fileHits.value.length) {
      selected.value = 0;
    }
  } catch (cause) {
    if (current !== request) return;
    indexing.value = false;
    searching.value = false;
    error.value = String(cause);
  }
}

async function act(command: "launch" | "launch_admin" | "reveal", id: number) {
  menu.value = null;
  error.value = "";
  try {
    if (command === "reveal") {
      await invoke(command, { id });
    } else {
      await invoke(command, { id, query: query.value });
    }
    // 输入框交给 hidden/shown 事件清空：启动要等一会儿，这里再清一次会抹掉再次唤出后已经输入的内容。
  } catch (cause) {
    error.value = String(cause);
  }
}

async function openFile(id: number) {
  menu.value = null;
  error.value = "";
  try {
    await invoke("open_file", { id });
  } catch (cause) {
    error.value = String(cause);
  }
}

async function revealFile(id: number) {
  menu.value = null;
  error.value = "";
  try {
    await invoke("reveal_file", { id });
  } catch (cause) {
    error.value = String(cause);
  }
}

async function openSelected() {
  const row = rows.value[selected.value];
  if (!row) return;
  if (row.kind === "app") {
    await act("launch", row.id);
  } else {
    await openFile(row.id);
  }
}

async function openMenu(index: number, event: MouseEvent) {
  const hit = results.value[index];
  if (!hit) return;
  selected.value = index;
  menu.value = { kind: "app", id: hit.id, x: event.clientX, y: event.clientY };
  await invoke("set_launcher_frame", { mode: "open" }).catch(() => undefined);
  await placeMenu();
}

async function openFileMenu(index: number, event: MouseEvent) {
  const hit = fileHits.value[index];
  if (!hit) return;
  selected.value = appCount.value + index;
  menu.value = { kind: "file", id: hit.id, x: event.clientX, y: event.clientY };
  await invoke("set_launcher_frame", { mode: "open" }).catch(() => undefined);
  await placeMenu();
}

function onPointerDown(event: PointerEvent) {
  const target = event.target;
  if (!(target instanceof Node)) return;
  if (menuEl.value?.contains(target)) return;
  if (sheetEl.value?.contains(target)) {
    if (event.button === 2) return;
    menu.value = null;
    return;
  }
  void invoke("hide_window");
}

async function placeAt(node: HTMLElement | null, point: { x: number; y: number } | null) {
  await nextTick();
  if (!node || !point) return null;
  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
  const margin = 8;
  const width = node.offsetWidth;
  const height = node.offsetHeight;
  if (width === 0 || height === 0) return null;
  let x = point.x;
  let y = point.y;
  if (x + width > window.innerWidth - margin) {
    x = window.innerWidth - width - margin;
  }
  if (x < margin) x = margin;
  if (y + height > window.innerHeight - margin) {
    y = window.innerHeight - height - margin;
  }
  if (y < margin) y = margin;
  return { x, y };
}

async function placeMenu() {
  const placed = await placeAt(menuEl.value, menu.value);
  if (!placed || !menu.value) return;
  menu.value = { ...menu.value, x: placed.x, y: placed.y };
}

function onKeydown(event: KeyboardEvent) {
  if (recording.value) return;
  if (event.key === "Escape") {
    event.preventDefault();
    if (menu.value) {
      menu.value = null;
      return;
    }
    if (settingsOpen.value) {
      closeSettings();
      void focusInput();
      return;
    }
    void invoke("hide_window");
    return;
  }
  if (settingsOpen.value) return;
  if (event.key === "ArrowDown") {
    event.preventDefault();
    if (rows.value.length === 0) return;
    selected.value = (selected.value + 1) % rows.value.length;
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    if (rows.value.length === 0) return;
    selected.value = (selected.value + rows.value.length - 1) % rows.value.length;
  } else if (event.key === "Enter") {
    event.preventDefault();
    void openSelected();
  }
}

function onFilesChange(enabled: boolean) {
  fileSearchOn.value = enabled;
  scheduleFileRefresh();
}

function onFileTypesChange(count: number) {
  fileTypeCount.value = count;
  scheduleFileRefresh();
}

function dismissUpdate() {
  const current = update.value;
  update.value = null;
  if (current) {
    void invoke("dismiss_update", { version: current.latest }).catch(() => undefined);
  }
}

async function focusInput() {
  await nextTick();
  inputEl.value?.focus();
  inputEl.value?.select();
}

function resetChrome(options: { preserveResume?: boolean } = {}) {
  window.clearTimeout(resultsClearTimer);
  if (!options.preserveResume) {
    resumeSettings.value = false;
  }
  settingsOpen.value = false;
  recording.value = false;
  clearSearch(true);
  window.clearTimeout(frameTimer);
  void invoke("set_launcher_frame", { mode: "idle" }).catch(() => undefined);
}

function restoreChrome() {
  window.clearTimeout(resultsClearTimer);
  clearSearch(true);
  menu.value = null;
  recording.value = false;
  if (resumeSettings.value) {
    warmSettingsPanel();
    settingsOpen.value = true;
  } else {
    settingsOpen.value = false;
  }
}

const stops: UnlistenFn[] = [];
let stopThemeWatch: (() => void) | null = null;

onMounted(async () => {
  window.addEventListener("pointerdown", onPointerDown);
  try {
    stops.push(
      await listen("shown", async () => {
        restoreChrome();
        await focusInput();
      }),
    );
    stops.push(
      await listen("hidden", () => {
        // 设置开着时点到桌面：先记住，下次唤出仍进设置。
        resumeSettings.value = settingsOpen.value;
        resetChrome({ preserveResume: true });
      }),
    );
    stops.push(
      await listen("open-settings", async () => {
        resumeSettings.value = false;
        openSettingsPanel();
        await nextTick();
      }),
    );
    stops.push(
      await listen("catalog-ready", () => {
        indexing.value = false;
        searching.value = false;
        results.value = [];
        answered.value = "";
        selected.value = 0;
        if (!settingsOpen.value && query.value.trim()) scheduleRefresh();
      }),
    );
    stops.push(
      await listen("files-ready", () => {
        if (!fileWanted.value) return;
        window.clearTimeout(fileTimer);
        void refreshFiles();
      }),
    );
    stops.push(
      await listen<{ latest: string; url: string }>("update-available", (event) => {
        update.value = event.payload;
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
  // 空闲时预挂载设置面板，第一次点齿轮就只做展开动画。
  settingsWarmTimer = window.setTimeout(() => {
    warmSettingsPanel();
  }, 600);
});

onUnmounted(() => {
  window.clearTimeout(searchTimer);
  window.clearTimeout(fileTimer);
  window.clearTimeout(frameTimer);
  window.clearTimeout(settingsWarmTimer);
  window.clearTimeout(resultsClearTimer);
  window.removeEventListener("pointerdown", onPointerDown);
  for (const stop of stops) stop();
  stopThemeWatch?.();
});
</script>

<template>
  <main class="stage">
    <section ref="sheetEl" class="sheet">
      <div class="query-row">
        <input
          ref="inputEl"
          class="query"
          spellcheck="false"
          :value="query"
          :placeholder="placeholder"
          :readonly="settingsOpen"
          @input="onQueryInput"
          @keydown="onKeydown"
          @focus="onInputFocus"
        />
        <button
          type="button"
          class="gear"
          :class="{ on: settingsOpen }"
          :aria-label="t.settings"
          :title="t.settings"
          @mousedown.prevent
          @click="toggleSettings"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path
              d="M19.14 12.94c.04-.31.06-.63.06-.94s-.02-.63-.06-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.49.49 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.48.48 0 0 0-.48-.41h-3.84a.48.48 0 0 0-.48.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 0 0-.59.22L2.74 8.87a.48.48 0 0 0 .12.61l2.03 1.58c-.04.31-.06.63-.06.94s.02.63.06.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.48-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.48.48 0 0 0-.12-.61l-2.03-1.58zM12 15.6A3.6 3.6 0 1 1 12 8.4a3.6 3.6 0 0 1 0 7.2z"
            />
          </svg>
          <span v-if="update" class="gear-dot" aria-hidden="true"></span>
        </button>
      </div>
      <div class="drawer" :class="{ open: drawerOpen }">
        <div>
          <SettingsPanel
            v-if="settingsMounted"
            v-show="settingsOpen"
            ref="panelEl"
            :language="language"
            :theme="theme"
            :hint="hint"
            :active="settingsOpen"
            :update="update"
            @language-change="language = $event"
            @theme-change="theme = $event"
            @hint-change="hint = $event"
            @recording-change="recording = $event"
            @files-change="onFilesChange"
            @file-types-change="onFileTypesChange"
            @update-dismiss="dismissUpdate"
          />
          <template v-if="!settingsOpen">
            <ul v-if="rows.length" class="results">
              <li v-for="(item, index) in results" :key="`app-${item.id}`">
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
              <template v-if="fileHits.length">
                <li class="section" aria-hidden="true">
                  <span>{{ t.filesSection }}</span>
                </li>
                <li v-for="(item, index) in fileHits" :key="`file-${item.id}`">
                  <button
                    type="button"
                    :class="{ active: appCount + index === selected }"
                    @mouseenter="selected = appCount + index"
                    @mousedown.prevent
                    @click="openFile(item.id)"
                    @contextmenu.prevent="openFileMenu(index, $event)"
                  >
                    <img v-if="item.icon" class="icon" :src="item.icon" alt="" />
                    <span v-else class="icon placeholder"></span>
                    <span class="name">{{ item.name }}</span>
                  </button>
                </li>
              </template>
            </ul>
            <p v-else-if="note" class="note" :class="{ error: note.error }">{{ note.text }}</p>
          </template>
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
      <template v-if="menu.kind === 'file'">
        <button type="button" @click="openFile(menu.id)">{{ t.open }}</button>
        <button type="button" @click="revealFile(menu.id)">{{ t.folder }}</button>
      </template>
      <template v-else>
        <button type="button" @click="act('launch', menu.id)">{{ t.open }}</button>
        <button type="button" @click="act('launch_admin', menu.id)">{{ t.admin }}</button>
        <button type="button" @click="act('reveal', menu.id)">{{ t.folder }}</button>
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
  --gear: #9a9a9a;
  --gear-on: #f3f3f3;
  --panel-text: #d0d0d0;
  --panel-strong: #ffffff;
  --panel-muted: #a3a3a3;
  --panel-menu: #d4d4d4;
  --panel-indicator: #3a3a3a;
  --panel-row: #242424;
  --panel-choice: #333333;
  --panel-line: #3a3a3a;
  --panel-key: #333333;
  --panel-switch: #4a4a4a;
  --panel-switch-on: #f2f2f2;
  --panel-knob: #f2f2f2;
  --panel-knob-on: #1c1c1c;
  --shadow:
    0 1px 3px rgba(0, 0, 0, 0.24),
    0 6px 16px rgba(0, 0, 0, 0.3);
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
  --gear: #b0b0b0;
  --gear-on: #5c5c5c;
  --panel-text: #2a2a2a;
  --panel-strong: #1a1a1a;
  --panel-muted: #8a8a8a;
  --panel-menu: #4a4a4a;
  --panel-indicator: #f3f3f3;
  --panel-row: #f6f6f6;
  --panel-choice: #f3f3f3;
  --panel-line: #ececec;
  --panel-key: #fafafa;
  --panel-switch: #dedede;
  --panel-switch-on: #2a2a2a;
  --panel-knob: #ffffff;
  --panel-knob-on: #ffffff;
  --shadow:
    0 1px 3px rgba(0, 0, 0, 0.04),
    0 6px 16px rgba(0, 0, 0, 0.07);
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

.query-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding-right: 16px;
}

.query {
  flex: 1;
  min-width: 0;
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
  padding: 0 16px 0 32px;
  caret-color: var(--caret);
  user-select: text;
}

.query:read-only {
  caret-color: transparent;
}

.gear {
  position: relative;
  width: 40px;
  height: 40px;
  flex: none;
  display: grid;
  place-items: center;
  border: 0;
  border-radius: 10px;
  background: transparent;
  color: var(--gear);
  cursor: pointer;
}

.gear-dot {
  position: absolute;
  top: 6px;
  right: 6px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #f5b800;
  pointer-events: none;
}

.gear svg {
  width: 22px;
  height: 22px;
  fill: currentColor;
}

.gear:hover,
.gear.on {
  color: var(--gear-on);
  background: var(--result-bg);
}

.sheet:has(.drawer.open) .query-row {
  padding-bottom: 10px;
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

.drawer.open > div::before {
  content: "";
  display: block;
  width: calc(100% - 40px);
  height: 1px;
  margin: 0 auto 12px;
  background: var(--line);
}

.drawer:not(.open) > div {
  pointer-events: none;
}

.query::placeholder {
  color: var(--query);
}

.results {
  list-style: none;
  margin: 0;
  padding: 6px 8px 8px;
  max-height: 400px;
  overflow-y: auto;
}

.results li.section {
  padding: 8px 18px 4px;
  color: var(--note);
  font-size: 12px;
  letter-spacing: 0.02em;
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
  z-index: 20;
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

@media (prefers-reduced-motion: reduce) {
  .drawer {
    transition: none;
  }
}
</style>
