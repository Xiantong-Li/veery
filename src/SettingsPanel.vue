<script lang="ts">
import type { SettingsInfo } from "./prefs";

let cachedInfo: SettingsInfo | null = null;
</script>

<script setup lang="ts">
import { computed, inject, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import logo from "./assets/Veery-Log_compressed.png";
import {
  applyTheme,
  copy,
  giteeRepo,
  githubRepo,
  languageOf,
  prefsKey,
  themeOf,
  type Language,
  type SettingsInfo,
  type ThemeChoice,
  type UpdateInfo,
} from "./prefs";

const props = defineProps<{
  language: Language;
  theme: ThemeChoice;
  hint: string;
}>();

const emit = defineEmits<{
  languageChange: [Language];
  themeChange: [ThemeChoice];
  hintChange: [string];
  recordingChange: [boolean];
}>();

const page = ref<"general" | "about">("general");
const menuEl = ref<HTMLElement | null>(null);
const generalEl = ref<HTMLButtonElement | null>(null);
const aboutEl = ref<HTMLButtonElement | null>(null);
const selectionReady = ref(false);
const selection = ref({ y: 0, height: 0 });
const info = ref<SettingsInfo | null>(cachedInfo ?? inject(prefsKey, null));
const switchReady = ref(false);
const recording = ref(false);
const message = ref("");
const checking = ref(false);
const updateState = ref<"idle" | "checking" | "current" | "available" | "failed">("idle");
const latestVersion = ref("");
const updateUrl = ref("");
const hintDraft = ref(props.hint);

const t = computed(() => copy(props.language));
const updateText = computed(() => {
  if (updateState.value === "checking") return t.value.checking;
  if (updateState.value === "current") return t.value.current;
  if (updateState.value === "available") return t.value.available(latestVersion.value);
  if (updateState.value === "failed") return t.value.failed;
  return "";
});
const selectionStyle = computed(() => ({
  height: `${selection.value.height}px`,
  transform: `translate3d(0, ${selection.value.y}px, 0)`,
}));

watch(
  () => props.hint,
  (value) => {
    hintDraft.value = value;
  },
);

watch(recording, (value) => emit("recordingChange", value));

watch(page, () => {
  void nextTick(moveSelection);
});

watch(
  () => props.language,
  () => {
    void nextTick(moveSelection);
  },
);

function moveSelection() {
  const button = page.value === "general" ? generalEl.value : aboutEl.value;
  if (!button) return;
  selection.value = { y: button.offsetTop, height: button.offsetHeight };
}

async function load() {
  const next = await invoke<SettingsInfo>("get_settings");
  cachedInfo = next;
  info.value = next;
  hintDraft.value = next.hint;
  emit("hintChange", next.hint);
  emit("languageChange", languageOf(next.language));
  emit("themeChange", themeOf(next.theme));
  await nextTick();
  requestAnimationFrame(() => {
    switchReady.value = true;
  });
}

let autostartWrite = Promise.resolve();
let autostartTicket = 0;
let fastAutostartTicket = 0;

function toggleAutostart() {
  if (!info.value) return;
  const enabled = !info.value.autostart;
  const previousFast = info.value.fastAutostart;
  const mine = ++autostartTicket;
  info.value.autostart = enabled;
  if (!enabled) info.value.fastAutostart = false;
  cachedInfo = info.value;
  message.value = "";
  autostartWrite = autostartWrite.then(async () => {
    try {
      await invoke("set_autostart", { enabled });
      if (mine !== autostartTicket || !info.value) return;
      info.value.autostart = enabled;
      if (!enabled) info.value.fastAutostart = false;
      cachedInfo = info.value;
      if (enabled) await load();
    } catch (cause) {
      if (mine === autostartTicket && info.value) {
        info.value.autostart = !enabled;
        info.value.fastAutostart = previousFast;
        cachedInfo = info.value;
        message.value = String(cause);
      }
    }
  });
}

function toggleFastAutostart() {
  if (!info.value?.autostart) return;
  const enabled = !info.value.fastAutostart;
  const mine = ++fastAutostartTicket;
  info.value.fastAutostart = enabled;
  cachedInfo = info.value;
  message.value = "";
  autostartWrite = autostartWrite.then(async () => {
    try {
      await invoke("set_fast_autostart", { enabled });
      if (mine === fastAutostartTicket && info.value) {
        info.value.fastAutostart = enabled;
        cachedInfo = info.value;
      }
    } catch (cause) {
      if (mine === fastAutostartTicket && info.value) {
        info.value.fastAutostart = !enabled;
        cachedInfo = info.value;
        message.value = String(cause);
      }
    }
  });
}

async function chooseLanguage(next: Language) {
  if (props.language === next) return;
  emit("languageChange", next);
  document.documentElement.lang = next;
  message.value = "";
  try {
    await invoke("set_language", { language: next });
    await load();
  } catch (cause) {
    message.value = String(cause);
  }
}

async function chooseTheme(next: ThemeChoice) {
  if (props.theme === next) return;
  emit("themeChange", next);
  applyTheme(next);
  message.value = "";
  try {
    await invoke("set_theme", { theme: next });
    if (info.value) info.value.theme = next;
  } catch (cause) {
    message.value = String(cause);
  }
}

async function saveHint() {
  const previous = props.hint;
  const next = hintDraft.value.trim();
  message.value = "";
  try {
    const saved = await invoke<string>("set_search_hint", { hint: next });
    hintDraft.value = saved;
    emit("hintChange", saved);
  } catch (cause) {
    hintDraft.value = previous;
    message.value = String(cause);
  }
}

async function checkUpdate() {
  checking.value = true;
  updateState.value = "checking";
  updateUrl.value = "";
  try {
    const result = await invoke<UpdateInfo>("check_update");
    latestVersion.value = result.latest.replace(/^v/, "");
    updateState.value = result.available ? "available" : "current";
    updateUrl.value = result.available ? result.url : "";
  } catch {
    updateState.value = "failed";
  } finally {
    checking.value = false;
  }
}

async function openLink(url: string) {
  message.value = "";
  try {
    await invoke("open_link", { url });
  } catch {
    message.value = t.value.linkFailed;
  }
}

async function startRecording() {
  message.value = "";
  recording.value = true;
  await invoke("pause_shortcut");
}

async function stopRecording() {
  if (!recording.value) return;
  recording.value = false;
  await invoke("restore_shortcut");
}

function sameShortcut(next: string, current: string) {
  const token = (part: string) => {
    switch (part.trim().toUpperCase()) {
      case "CONTROL":
        return "CTRL";
      case "META":
        return "SUPER";
      default:
        return part.trim().toUpperCase();
    }
  };
  const fold = (value: string) =>
    value
      .split("+")
      .map(token)
      .filter(Boolean)
      .sort()
      .join("+");
  return fold(next) === fold(current);
}

async function onKeydown(event: KeyboardEvent) {
  if (!recording.value) return;
  event.preventDefault();
  event.stopPropagation();
  if (event.key === "Escape") {
    await stopRecording();
    return;
  }
  if (
    event.code.startsWith("Control") ||
    event.code.startsWith("Alt") ||
    event.code.startsWith("Shift") ||
    event.code.startsWith("Meta")
  ) {
    return;
  }
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Super");
  if (parts.length === 0) {
    message.value = t.value.needModifier;
    return;
  }
  parts.push(event.code);
  const next = parts.join("+");
  if (info.value && sameShortcut(next, info.value.shortcut)) {
    message.value = t.value.duplicateShortcut;
    await stopRecording();
    return;
  }
  recording.value = false;
  try {
    const label = await invoke<string>("set_shortcut", { shortcut: next });
    if (info.value) {
      info.value.shortcut = next;
      info.value.shortcutLabel = label;
    }
    message.value = "";
  } catch (cause) {
    message.value = String(cause);
    await invoke("restore_shortcut");
  }
}

const stops: UnlistenFn[] = [];
let menuObserver: ResizeObserver | null = null;

onMounted(() => {
  window.addEventListener("keydown", onKeydown, true);
  void nextTick(() => {
    moveSelection();
    requestAnimationFrame(() => {
      selectionReady.value = true;
    });
  });
  if (menuEl.value && typeof ResizeObserver !== "undefined") {
    menuObserver = new ResizeObserver(() => moveSelection());
    menuObserver.observe(menuEl.value);
  }
  void load().catch((cause: unknown) => {
    message.value = String(cause);
  });
  void listen<{ ctrl: boolean; shift: boolean; alt: boolean }>("alt-space", (event) => {
    if (!recording.value) return;
    void onKeydown(
      new KeyboardEvent("keydown", {
        key: " ",
        code: "Space",
        ctrlKey: event.payload.ctrl,
        shiftKey: event.payload.shift,
        altKey: event.payload.alt,
        cancelable: true,
      }),
    );
  }).then((stop) => stops.push(stop));
});

onUnmounted(() => {
  menuObserver?.disconnect();
  window.removeEventListener("keydown", onKeydown, true);
  for (const stop of stops) stop();
  void stopRecording();
});

defineExpose({
  stopRecording,
  get recording() {
    return recording.value;
  },
});
</script>

<template>
  <div class="settings">
    <aside>
      <img class="brand" :src="logo" alt="Veery" />
      <nav ref="menuEl" class="sidebar">
        <span class="indicator" :class="{ ready: selectionReady }" :style="selectionStyle"></span>
        <button
          ref="generalEl"
          type="button"
          :class="{ current: page === 'general' }"
          @click="page = 'general'"
        >
          {{ t.general }}
        </button>
        <button ref="aboutEl" type="button" :class="{ current: page === 'about' }" @click="page = 'about'">
          {{ t.about }}
        </button>
      </nav>
    </aside>

    <section v-if="page === 'general'" class="pane">
      <h1>{{ t.general }}</h1>
      <div class="row">
        <div>
          <p class="title">{{ t.autostart }}</p>
          <p class="desc">{{ t.autostartDesc }}</p>
        </div>
        <button
          type="button"
          class="switch"
          :class="{ on: info?.autostart, ready: switchReady }"
          :aria-pressed="info?.autostart"
          @click="toggleAutostart"
        >
          <span></span>
        </button>
      </div>
      <div class="row" :class="{ dim: !info?.autostart }">
        <div>
          <p class="title">{{ t.fastAutostart }}</p>
          <p class="desc">{{ t.fastAutostartDesc }}</p>
        </div>
        <button
          type="button"
          class="switch"
          :class="{ on: info?.fastAutostart, muted: !info?.autostart, ready: switchReady }"
          :aria-pressed="info?.fastAutostart"
          :disabled="!info?.autostart"
          @click="toggleFastAutostart"
        >
          <span></span>
        </button>
      </div>
      <div class="row">
        <div>
          <p class="title">{{ t.shortcut }}</p>
          <p class="desc">{{ t.shortcutDesc }}</p>
        </div>
        <button type="button" class="key" :class="{ recording }" @click="startRecording">
          {{ recording ? t.pressShortcut : info?.shortcutLabel || t.loading }}
        </button>
      </div>
      <div class="row">
        <div>
          <p class="title">{{ t.hint }}</p>
          <p class="desc">{{ t.hintDesc }}</p>
        </div>
        <input
          v-model="hintDraft"
          class="hint"
          maxlength="20"
          spellcheck="false"
          :placeholder="t.search"
          @keydown.enter.prevent="saveHint"
          @blur="saveHint"
        />
      </div>
      <div class="row">
        <div>
          <p class="title">{{ t.language }}</p>
          <p class="desc">{{ t.languageDesc }}</p>
        </div>
        <div class="choices">
          <button type="button" :class="{ on: language === 'zh' }" @click="chooseLanguage('zh')">中文</button>
          <button type="button" :class="{ on: language === 'en' }" @click="chooseLanguage('en')">English</button>
        </div>
      </div>
      <div class="row">
        <div>
          <p class="title">{{ t.theme }}</p>
          <p class="desc">{{ t.themeDesc }}</p>
        </div>
        <div class="choices">
          <button type="button" :class="{ on: theme === 'system' }" @click="chooseTheme('system')">
            {{ t.system }}
          </button>
          <button type="button" :class="{ on: theme === 'light' }" @click="chooseTheme('light')">
            {{ t.light }}
          </button>
          <button type="button" :class="{ on: theme === 'dark' }" @click="chooseTheme('dark')">
            {{ t.dark }}
          </button>
        </div>
      </div>
      <p v-if="message" class="message">{{ message }}</p>
    </section>

    <section v-else class="pane">
      <h1>{{ t.about }}</h1>
      <p class="name">{{ info?.name || "Veery" }}</p>
      <p class="summary">{{ t.summary }}</p>
      <dl>
        <div>
          <dt>{{ t.author }}</dt>
          <dd>{{ info?.author }}</dd>
        </div>
        <div>
          <dt>{{ t.version }}</dt>
          <dd>{{ info?.version }}</dd>
        </div>
        <div>
          <dt>GitHub</dt>
          <dd>
            <button type="button" class="link" @click="openLink(githubRepo)">github.com/Xiantong-Li/veery</button>
          </dd>
        </div>
        <div>
          <dt>Gitee</dt>
          <dd>
            <button type="button" class="link" @click="openLink(giteeRepo)">gitee.com/li-xiantong/veery</button>
          </dd>
        </div>
      </dl>
      <div class="update">
        <button type="button" class="key" :disabled="checking" @click="checkUpdate">
          {{ checking ? t.checking : t.check }}
        </button>
        <p v-if="updateText" class="desc">{{ updateText }}</p>
        <button v-if="updateUrl" type="button" class="link" @click="openLink(updateUrl)">{{ t.download }}</button>
      </div>
      <p v-if="message" class="message">{{ message }}</p>
    </section>
  </div>
</template>

<style scoped>
.settings {
  display: flex;
  height: 400px;
  color: var(--panel-text);
  background: var(--sheet);
}

aside {
  box-sizing: border-box;
  width: 168px;
  flex: none;
  padding: 18px 10px;
  background: transparent;
  border-right: 1px solid var(--panel-line);
}

.brand {
  display: block;
  width: 118px;
  height: auto;
  margin: 0 6px 16px;
}

.sidebar {
  position: relative;
}

.indicator {
  position: absolute;
  z-index: 0;
  left: 0;
  right: 0;
  top: 0;
  border-radius: 8px;
  background: var(--panel-indicator);
  opacity: 0;
  pointer-events: none;
}

.indicator.ready {
  opacity: 1;
  transition:
    transform 0.46s cubic-bezier(0.32, 0.72, 0, 1),
    height 0.46s cubic-bezier(0.32, 0.72, 0, 1);
}

aside button {
  position: relative;
  z-index: 1;
  display: block;
  width: 100%;
  margin: 0 0 4px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--panel-menu);
  font: inherit;
  font-size: 14px;
  text-align: left;
  padding: 10px 12px;
  cursor: pointer;
  transition: color 0.35s ease;
}

aside button.current {
  color: var(--panel-strong);
}

.pane {
  flex: 1;
  min-width: 0;
  padding: 22px 28px 20px;
  overflow: auto;
}

h1 {
  margin: 0 0 18px;
  color: var(--panel-strong);
  font-size: 20px;
  font-weight: 600;
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  margin: 0 0 14px;
  padding: 14px 16px;
  border-radius: 12px;
  background: var(--panel-row);
}

.row.dim {
  opacity: 0.55;
}

.title {
  margin: 0 0 4px;
  color: var(--panel-strong);
  font-size: 14px;
}

.desc,
.summary {
  margin: 0;
  color: var(--panel-muted);
  font-size: 12px;
  line-height: 1.55;
}

.switch {
  width: 44px;
  height: 26px;
  flex: none;
  position: relative;
  border: 0;
  border-radius: 99px;
  background: var(--panel-switch);
  cursor: pointer;
}

.switch span {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: var(--panel-knob);
}

.switch.ready span {
  transition: transform 0.15s ease;
}

.switch.on {
  background: var(--panel-switch-on);
}

.switch.on span {
  background: var(--panel-knob-on);
  transform: translateX(18px);
}

.switch.muted,
.switch:disabled {
  opacity: 0.45;
  cursor: default;
}

.key {
  flex: none;
  min-width: 132px;
  border: 1px solid var(--panel-line);
  border-radius: 8px;
  background: var(--panel-key);
  color: var(--panel-strong);
  font: inherit;
  font-size: 13px;
  padding: 8px 14px;
  cursor: pointer;
}

.key:disabled {
  cursor: default;
  opacity: 0.7;
}

.key.recording {
  border-color: var(--panel-strong);
}

.hint {
  width: 168px;
  flex: none;
  box-sizing: border-box;
  border: 1px solid var(--panel-line);
  border-radius: 8px;
  background: var(--panel-key);
  color: var(--panel-strong);
  font: inherit;
  font-size: 13px;
  padding: 8px 12px;
  outline: none;
}

.choices {
  display: flex;
  flex: none;
  padding: 3px;
  border: 1px solid var(--panel-line);
  border-radius: 8px;
  background: var(--panel-choice);
}

.choices button {
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--panel-muted);
  font: inherit;
  font-size: 13px;
  padding: 6px 10px;
  cursor: pointer;
}

.choices button.on {
  background: var(--sheet);
  color: var(--panel-strong);
}

.message {
  margin: 0;
  color: var(--error);
  font-size: 13px;
}

.name {
  margin: 0 0 8px;
  color: var(--panel-strong);
  font-size: 26px;
  font-weight: 600;
}

.summary {
  max-width: 520px;
  margin-bottom: 18px;
}

dl {
  margin: 0 0 18px;
}

dl div {
  display: flex;
  gap: 16px;
  margin: 0 0 10px;
}

dt {
  width: 72px;
  flex: none;
  color: var(--panel-muted);
  font-size: 13px;
}

dd {
  margin: 0;
  min-width: 0;
  color: var(--panel-strong);
  font-size: 13px;
}

.link {
  border: 0;
  padding: 0;
  background: none;
  color: inherit;
  font: inherit;
  text-align: left;
  text-decoration: underline;
  cursor: pointer;
}

.update {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 14px;
}

@media (prefers-reduced-motion: reduce) {
  .indicator.ready,
  aside button {
    transition: none;
  }
}
</style>
