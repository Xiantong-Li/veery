<script setup lang="ts">
import { computed, inject, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import logo from "./assets/Veery-Log_compressed.png";
import {
  applyTheme,
  copy,
  giteeRepo,
  githubRepo,
  languageOf,
  prefsKey,
  themeOf,
  watchSystemTheme,
  type Language,
  type SettingsInfo,
  type ThemeChoice,
  type UpdateInfo,
} from "./prefs";

const page = ref<"general" | "about">("general");
const menuEl = ref<HTMLElement | null>(null);
const generalEl = ref<HTMLButtonElement | null>(null);
const aboutEl = ref<HTMLButtonElement | null>(null);
const selectionReady = ref(false);
const selection = ref({ y: 0, height: 0 });
const initial = inject(prefsKey, null);
const info = ref<SettingsInfo | null>(initial);
const language = ref<Language>(languageOf(initial?.language));
const theme = ref<ThemeChoice>(themeOf(initial?.theme));
const recording = ref(false);
const message = ref("");
const checking = ref(false);
const updateState = ref<"idle" | "checking" | "current" | "available" | "failed">("idle");
const latestVersion = ref("");
const updateUrl = ref("");

const t = computed(() => copy(language.value));
const updateText = computed(() => {
  if (updateState.value === "checking") return t.value.checking;
  if (updateState.value === "current") return t.value.current;
  if (updateState.value === "available") return t.value.available(latestVersion.value);
  if (updateState.value === "failed") return t.value.failed;
  return "";
});

async function load() {
  info.value = await invoke<SettingsInfo>("get_settings");
  language.value = languageOf(info.value.language);
  theme.value = themeOf(info.value.theme);
  applyTheme(theme.value);
  document.documentElement.lang = language.value;
}

let autostartWrite = Promise.resolve();
let autostartTicket = 0;

function toggleAutostart() {
  if (!info.value) return;
  const enabled = !info.value.autostart;
  const mine = ++autostartTicket;
  info.value.autostart = enabled;
  message.value = "";
  autostartWrite = autostartWrite.then(async () => {
    try {
      await invoke("set_autostart", { enabled });
      if (mine === autostartTicket && info.value) info.value.autostart = enabled;
    } catch (cause) {
      if (mine === autostartTicket && info.value) {
        info.value.autostart = !enabled;
        message.value = String(cause);
      }
    }
  });
}

async function chooseLanguage(next: Language) {
  if (language.value === next) return;
  language.value = next;
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
  if (theme.value === next) return;
  theme.value = next;
  applyTheme(next);
  message.value = "";
  try {
    await invoke("set_theme", { theme: next });
    if (info.value) info.value.theme = next;
  } catch (cause) {
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
    if (result.available) {
      updateState.value = "available";
      updateUrl.value = result.url;
    } else {
      updateState.value = "current";
    }
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
  recording.value = false;
  await invoke("restore_shortcut");
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

const selectionStyle = computed(() => ({
  height: `${selection.value.height}px`,
  transform: `translate3d(0, ${selection.value.y}px, 0)`,
}));

function moveSelection() {
  const button = page.value === "general" ? generalEl.value : aboutEl.value;
  if (!button) return;
  selection.value = { y: button.offsetTop, height: button.offsetHeight };
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

let menuObserver: ResizeObserver | null = null;
let stopAltSpace: (() => void) | null = null;
let stopAppearance: (() => void) | null = null;
let stopThemeWatch: (() => void) | null = null;

watch(page, () => {
  void nextTick(moveSelection);
});

watch(language, () => {
  void nextTick(moveSelection);
});

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  stopThemeWatch = watchSystemTheme(() => theme.value);
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
  void listen<{ ctrl: boolean; shift: boolean; alt: boolean }>("alt-space", (event) => {
    if (!recording.value) return;
    const mods = event.payload;
    void onKeydown(
      new KeyboardEvent("keydown", {
        key: " ",
        code: "Space",
        ctrlKey: mods.ctrl,
        shiftKey: mods.shift,
        altKey: mods.alt,
        cancelable: true,
      }),
    );
  })
    .then((stop) => {
      stopAltSpace = stop;
    })
    .catch((cause: unknown) => {
      message.value = String(cause);
    });
  void listen<{ language: string; theme: string }>("appearance", (event) => {
    language.value = languageOf(event.payload.language);
    theme.value = themeOf(event.payload.theme);
    applyTheme(theme.value);
    document.documentElement.lang = language.value;
  })
    .then((stop) => {
      stopAppearance = stop;
    })
    .catch(() => undefined);
  if (!info.value) void load();
});

onUnmounted(() => {
  menuObserver?.disconnect();
  window.removeEventListener("keydown", onKeydown);
  stopAltSpace?.();
  stopAppearance?.();
  stopThemeWatch?.();
});
</script>

<template>
  <main class="settings">
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
    <section v-if="page === 'general'" class="panel">
      <h1>{{ t.general }}</h1>
      <div class="row">
        <div>
          <p class="title">{{ t.autostart }}</p>
          <p class="desc">{{ t.autostartDesc }}</p>
        </div>
        <button
          type="button"
          class="switch"
          :class="{ on: info?.autostart }"
          :aria-pressed="info?.autostart"
          @click="toggleAutostart"
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
    <section v-else class="panel">
      <h1>{{ t.about }}</h1>
      <p class="name">{{ info?.name }}</p>
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
  </main>
</template>

<style>
html,
body,
#app {
  height: 100%;
  margin: 0;
  background: var(--bg);
  color: var(--text);
}

html[data-theme="dark"] {
  color-scheme: dark;
  --bg: #1c1c1c;
  --surface: #2a2a2a;
  --line: #3a3a3a;
  --text: #f2f2f2;
  --muted: #a3a3a3;
  --strong: #ffffff;
  --menu: #d4d4d4;
  --indicator: #3a3a3a;
  --choice: #333333;
  --switch: #4a4a4a;
  --switch-on: #f2f2f2;
  --knob: #f2f2f2;
  --knob-on: #1c1c1c;
  --key-bg: #333333;
  --key-line: #4a4a4a;
  --key-text: #f2f2f2;
  --danger: #ffb4ab;
}

html[data-theme="light"],
html:not([data-theme]) {
  color-scheme: light;
  --bg: #f6f6f6;
  --surface: #ffffff;
  --line: #ececec;
  --text: #2a2a2a;
  --muted: #8a8a8a;
  --strong: #1a1a1a;
  --menu: #4a4a4a;
  --indicator: #f3f3f3;
  --choice: #f3f3f3;
  --switch: #dedede;
  --switch-on: #2a2a2a;
  --knob: #ffffff;
  --knob-on: #ffffff;
  --key-bg: #fafafa;
  --key-line: #e6e6e6;
  --key-text: #1f1f1f;
  --danger: #b42318;
}

body {
  font-family: "Segoe UI", "Microsoft YaHei", sans-serif;
  color: var(--text);
  -webkit-font-smoothing: antialiased;
}

.settings {
  display: flex;
  height: 100%;
  min-height: 100%;
}

aside {
  box-sizing: border-box;
  width: 180px;
  flex: none;
  padding: 22px 12px;
  background: var(--surface);
  border-right: 1px solid var(--line);
}

.brand {
  display: block;
  width: 132px;
  height: auto;
  margin: 0 8px 18px;
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
  background: var(--indicator);
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
  color: var(--menu);
  font: inherit;
  font-size: 14px;
  text-align: left;
  padding: 10px 12px;
  cursor: pointer;
  transition: color 0.35s ease;
}

aside button.current {
  color: var(--strong);
}

@media (prefers-reduced-motion: reduce) {
  .indicator.ready,
  aside button {
    transition: none;
  }
}

.panel {
  flex: 1;
  padding: 28px 32px;
  overflow: auto;
}

h1 {
  margin: 0 0 22px;
  font-size: 22px;
  font-weight: 600;
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  margin: 0 0 18px;
  padding: 16px 18px;
  border-radius: 12px;
  background: var(--surface);
}

.title {
  margin: 0 0 4px;
  font-size: 15px;
}

.desc,
.summary {
  margin: 0;
  color: var(--muted);
  font-size: 13px;
  line-height: 1.6;
}

.switch {
  width: 44px;
  height: 26px;
  flex: none;
  position: relative;
  border: 0;
  border-radius: 99px;
  background: var(--switch);
  cursor: pointer;
}

.switch span {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: var(--knob);
  transition: transform 0.15s ease;
}

.switch.on {
  background: var(--switch-on);
}

.switch.on span {
  background: var(--knob-on);
  transform: translateX(18px);
}

.key {
  flex: none;
  min-width: 132px;
  border: 1px solid var(--key-line);
  border-radius: 8px;
  background: var(--key-bg);
  color: var(--key-text);
  font: inherit;
  font-size: 14px;
  padding: 8px 14px;
  cursor: pointer;
}

.key:disabled {
  cursor: default;
  opacity: 0.7;
}

.key.recording {
  border-color: var(--strong);
}

.choices {
  display: flex;
  flex: none;
  padding: 3px;
  border: 1px solid var(--key-line);
  border-radius: 8px;
  background: var(--choice);
}

.choices button {
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 13px;
  padding: 6px 10px;
  cursor: pointer;
}

.choices button.on {
  background: var(--surface);
  color: var(--strong);
}

.message {
  margin: 0;
  color: var(--danger);
  font-size: 13px;
}

.name {
  margin: 0 0 10px;
  font-size: 28px;
  font-weight: 600;
}

.summary {
  max-width: 460px;
  margin-bottom: 22px;
}

dl {
  margin: 0 0 22px;
}

dl div {
  display: flex;
  gap: 16px;
  margin: 0 0 10px;
}

dt {
  width: 72px;
  flex: none;
  color: var(--muted);
}

dd {
  margin: 0;
  min-width: 0;
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
  gap: 14px;
}
</style>
