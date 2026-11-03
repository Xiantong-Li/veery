import { createApp } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";

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
  void import("./App.vue").then(({ default: App }) => {
    createApp(App).mount("#app");
  });
}
