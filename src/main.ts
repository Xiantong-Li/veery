import { createApp } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App.vue";

function isSettingsWindow() {
  try {
    return getCurrentWindow().label === "settings";
  } catch {
    return false;
  }
}

if (isSettingsWindow()) {
  void import("./Settings.vue").then(({ default: Settings }) => {
    createApp(Settings).mount("#app");
  });
} else {
  createApp(App).mount("#app");
}
