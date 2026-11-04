import type { InjectionKey } from "vue";

export type Language = "zh" | "en";
export type ThemeChoice = "system" | "light" | "dark";

export type SettingsInfo = {
  autostart: boolean;
  shortcut: string;
  shortcutLabel: string;
  language: string;
  theme: string;
  hint: string;
  name: string;
  version: string;
  author: string;
  summary: string;
};

export type UpdateInfo = {
  current: string;
  latest: string;
  available: boolean;
  url: string;
};

export const prefsKey: InjectionKey<SettingsInfo | null> = Symbol("prefs");

export const githubRepo = "https://github.com/Xiantong-Li/veery";
export const giteeRepo = "https://gitee.com/li-xiantong/veery";

export function languageOf(value: string | undefined): Language {
  return value === "en" ? "en" : "zh";
}

export function themeOf(value: string | undefined): ThemeChoice {
  if (value === "dark" || value === "system") return value;
  return "light";
}

export function applyTheme(choice: ThemeChoice) {
  const dark =
    choice === "dark" ||
    (choice === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  document.documentElement.style.colorScheme = dark ? "dark" : "light";
}

export function watchSystemTheme(getChoice: () => ThemeChoice) {
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  const update = () => {
    if (getChoice() === "system") applyTheme("system");
  };
  query.addEventListener("change", update);
  return () => query.removeEventListener("change", update);
}

const zh = {
  general: "常用设置",
  about: "关于",
  autostart: "开机启动",
  autostartDesc: "登录 Windows 后自动打开 Veery，并留在托盘里。",
  shortcut: "全局快捷键",
  shortcutDesc: "用来呼出或隐藏搜索框。点击右侧后按下新的组合键，Esc 取消。",
  pressShortcut: "请按快捷键",
  loading: "读取中",
  needModifier: "请带上 Ctrl、Alt 或 Shift",
  duplicateShortcut: "请勿录入重复的快捷键",
  language: "语言",
  languageDesc: "设置、搜索和托盘菜单使用的语言。",
  theme: "主题",
  themeDesc: "浅色、深色，或跟随 Windows。",
  system: "跟随系统",
  light: "浅色",
  dark: "深色",
  summary:
    "Veery 是一个轻量的 Windows 启动器。按下全局快捷键弹出搜索框，输入程序的全称、拼音或首字母，回车或点击即可打开开始菜单里的程序。",
  author: "作者",
  version: "版本号",
  source: "开源地址",
  check: "检测更新",
  checking: "正在检查…",
  current: "当前已是最新版本",
  available: (version: string) => `发现新版本 ${version}`,
  failed: "没有检查到更新，请稍后再试",
  download: "前往下载",
  linkFailed: "暂时打不开这个链接",
  search: "搜索",
  customizeHint: "自定义备注",
  resetHint: "恢复默认备注",
  indexing: "正在读取开始菜单",
  empty: "没有匹配的程序",
  open: "打开",
  admin: "以管理员身份运行",
  folder: "打开所在文件夹",
};

const en = {
  general: "General",
  about: "About",
  autostart: "Start with Windows",
  autostartDesc: "Open Veery when you sign in to Windows, and keep it in the tray.",
  shortcut: "Global shortcut",
  shortcutDesc: "Shows or hides the search box. Click the button, then press a new shortcut. Esc cancels.",
  pressShortcut: "Press a shortcut",
  loading: "Loading",
  needModifier: "Include Ctrl, Alt, or Shift",
  duplicateShortcut: "That shortcut is already in use",
  language: "Language",
  languageDesc: "Language used by settings, search, and the tray menu.",
  theme: "Theme",
  themeDesc: "Light, dark, or match Windows.",
  system: "System",
  light: "Light",
  dark: "Dark",
  summary:
    "Veery is a small Windows launcher. Press the global shortcut, type a program name, pinyin, or initials, then press Enter or click to open it.",
  author: "Author",
  version: "Version",
  source: "Source",
  check: "Check for updates",
  checking: "Checking…",
  current: "You're up to date",
  available: (version: string) => `Version ${version} is available`,
  failed: "Couldn't check for updates. Try again later.",
  download: "Download",
  linkFailed: "Couldn't open that link",
  search: "Search",
  customizeHint: "Customize hint",
  resetHint: "Reset hint",
  indexing: "Reading the Start menu",
  empty: "No matching programs",
  open: "Open",
  admin: "Run as administrator",
  folder: "Open file location",
};

export function copy(language: Language) {
  return language === "en" ? en : zh;
}
