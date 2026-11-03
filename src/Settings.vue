<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import logo from "./assets/Veery-Log_compressed.png";

type SettingsInfo = {
  autostart: boolean;
  shortcut: string;
  shortcutLabel: string;
  name: string;
  version: string;
  author: string;
  summary: string;
};

const page = ref<"general" | "about">("general");
const info = ref<SettingsInfo | null>(null);
const recording = ref(false);
const message = ref("");

async function load() {
  info.value = await invoke<SettingsInfo>("get_settings");
}

async function toggleAutostart() {
  if (!info.value) return;
  const enabled = !info.value.autostart;
  message.value = "";
  try {
    await invoke("set_autostart", { enabled });
    info.value.autostart = enabled;
  } catch (cause) {
    message.value = String(cause);
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
    message.value = "请带上 Ctrl、Alt 或 Shift";
    return;
  }
  parts.push(event.code);
  const next = parts.join("+");
  if (info.value && sameShortcut(next, info.value.shortcut)) {
    message.value = "请勿录入重复的快捷键";
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

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  void load();
});
</script>

<template>
  <main class="settings">
    <aside>
      <img class="brand" :src="logo" alt="Veery" />
      <button type="button" :class="{ current: page === 'general' }" @click="page = 'general'">
        常用设置
      </button>
      <button type="button" :class="{ current: page === 'about' }" @click="page = 'about'">
        关于
      </button>
    </aside>
    <section v-if="page === 'general'" class="panel">
      <h1>常用设置</h1>
      <div class="row">
        <div>
          <p class="title">开机启动</p>
          <p class="desc">登录 Windows 后自动打开 Veery，并留在托盘里。</p>
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
          <p class="title">全局快捷键</p>
          <p class="desc">用来呼出或隐藏搜索框。点击右侧后按下新的组合键，Esc 取消。</p>
        </div>
        <button type="button" class="key" :class="{ recording }" @click="startRecording">
          {{ recording ? "请按快捷键" : info?.shortcutLabel || "读取中" }}
        </button>
      </div>
      <p v-if="message" class="message">{{ message }}</p>
    </section>
    <section v-else class="panel">
      <h1>关于</h1>
      <p class="name">{{ info?.name }}</p>
      <p class="summary">{{ info?.summary }}</p>
      <dl>
        <div>
          <dt>作者</dt>
          <dd>{{ info?.author }}</dd>
        </div>
        <div>
          <dt>版本号</dt>
          <dd>{{ info?.version }}</dd>
        </div>
      </dl>
    </section>
  </main>
</template>

<style>
html,
body,
#app {
  height: 100%;
  margin: 0;
  background: #f6f6f6;
  color-scheme: light;
}

body {
  font-family: "Segoe UI", "Microsoft YaHei", sans-serif;
  color: #2a2a2a;
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
  background: #ffffff;
  border-right: 1px solid #ececec;
}

.brand {
  display: block;
  width: 132px;
  height: auto;
  margin: 0 8px 18px;
}

aside button {
  display: block;
  width: 100%;
  margin: 0 0 4px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: #4a4a4a;
  font: inherit;
  font-size: 14px;
  text-align: left;
  padding: 10px 12px;
  cursor: pointer;
}

aside button.current {
  background: #f3f3f3;
  color: #1a1a1a;
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
  background: #ffffff;
}

.title {
  margin: 0 0 4px;
  font-size: 15px;
}

.desc,
.summary {
  margin: 0;
  color: #8a8a8a;
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
  background: #dedede;
  cursor: pointer;
}

.switch span {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: #ffffff;
  transition: transform 0.15s ease;
}

.switch.on {
  background: #2a2a2a;
}

.switch.on span {
  transform: translateX(18px);
}

.key {
  flex: none;
  min-width: 132px;
  border: 1px solid #e6e6e6;
  border-radius: 8px;
  background: #fafafa;
  color: #1f1f1f;
  font: inherit;
  font-size: 14px;
  padding: 8px 14px;
  cursor: pointer;
}

.key.recording {
  border-color: #2a2a2a;
}

.message {
  margin: 0;
  color: #b42318;
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
  margin: 0;
}

dl div {
  display: flex;
  gap: 16px;
  margin: 0 0 10px;
}

dt {
  width: 64px;
  color: #8a8a8a;
}

dd {
  margin: 0;
}
</style>
