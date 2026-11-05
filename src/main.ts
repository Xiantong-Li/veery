import { createApp } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { applyTheme, languageOf, prefsKey, themeOf, type SettingsInfo } from "./prefs";
import App from "./App.vue";

async function boot() {
  const prefs = await invoke<SettingsInfo>("get_settings").catch(() => null);
  if (prefs) {
    applyTheme(themeOf(prefs.theme));
    document.documentElement.lang = languageOf(prefs.language);
  }
  const app = createApp(App);
  app.provide(prefsKey, prefs);
  app.mount("#app");
}

void boot();
