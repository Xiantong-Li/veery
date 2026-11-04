import { createApp, type Component } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyTheme, languageOf, prefsKey, themeOf, type SettingsInfo } from "./prefs";

function isSettingsWindow() {
  try {
    return getCurrentWindow().label === "settings";
  } catch {
    return false;
  }
}

function mount(page: Component, prefs: SettingsInfo | null) {
  if (prefs) {
    applyTheme(themeOf(prefs.theme));
    document.documentElement.lang = languageOf(prefs.language);
  }
  const app = createApp(page);
  app.provide(prefsKey, prefs);
  app.mount("#app");
}

function loadPrefs() {
  return invoke<SettingsInfo>("get_settings").catch(() => null);
}

async function boot() {
  // 两个 import() 必须分开写。合成一次三元导入时，安装包只会带上搜索页的样式。
  if (isSettingsWindow()) {
    const [page, prefs] = await Promise.all([import("./Settings.vue"), loadPrefs()]);
    mount(page.default, prefs);
    return;
  }
  const [page, prefs] = await Promise.all([import("./App.vue"), loadPrefs()]);
  mount(page.default, prefs);
}

void boot();
