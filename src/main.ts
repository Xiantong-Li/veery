import { createApp } from "vue";
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

async function boot() {
  const settings = isSettingsWindow();
  const [page, prefs] = await Promise.all([
    settings ? import("./Settings.vue") : import("./App.vue"),
    invoke<SettingsInfo>("get_settings").catch(() => null),
  ]);
  if (prefs) {
    applyTheme(themeOf(prefs.theme));
    document.documentElement.lang = languageOf(prefs.language);
  }
  const app = createApp(page.default);
  app.provide(prefsKey, prefs);
  app.mount("#app");
}

void boot();
