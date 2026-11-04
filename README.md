<p align="center">
  <img src="src/assets/Veery-Log_compressed.png" alt="Veery" width="200" />
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT License" /></a>
  <img src="https://img.shields.io/badge/platform-Windows-0078D6" alt="Windows" />
  <img src="https://img.shields.io/badge/version-0.2.0-lightgrey" alt="version 0.2.0" />
  <a href="https://github.com/Xiantong-Li/veery"><img src="https://img.shields.io/badge/GitHub-181717?logo=github&amp;logoColor=white" alt="GitHub" /></a>
  <a href="https://gitee.com/li-xiantong/veery"><img src="https://img.shields.io/badge/Gitee-C71D23?logo=gitee&amp;logoColor=white" alt="Gitee" /></a>
  <br />
  <a href="https://github.com/Xiantong-Li/veery/releases/latest">GitHub 下载安装版</a>
  ·
  <a href="https://gitee.com/li-xiantong/veery/releases/latest">码云下载安装版</a>
</p>

<h1 align="center">Veery</h1>

<p align="center"><strong>Launch Anything, Simply.</strong></p>

<p align="center">让一切启动，都简单一点。</p>

Veery（维里）是一个常驻系统托盘的 Windows 启动器。按下全局快捷键后，在屏幕上方弹出搜索框，输入程序的全称、拼音或首字母，回车或点击即可打开开始菜单里的程序。

## 功能

- 启动后留在托盘里，不占用任务栏
- 默认使用 `Alt + 空格` 呼出或隐藏搜索框，快捷键可以在设置里修改
- 按程序全称、拼音和首字母匹配开始菜单中的快捷方式
- 鼠标悬停选中结果，点击或按回车启动
- 右键结果可以打开、以管理员身份运行，或打开所在文件夹
- 支持开机启动

## 使用

程序启动后窗口先隐藏。左键点击托盘图标，或按下全局快捷键，即可呼出搜索框。

| 操作 | 结果 |
| --- | --- |
| 全局快捷键（默认 `Alt + 空格`） | 呼出或隐藏搜索框 |
| 输入文字 | 匹配开始菜单程序 |
| 鼠标悬停 | 选中该行 |
| 回车，或点击结果 | 启动程序 |
| 右键结果 | 打开、以管理员身份运行、打开所在文件夹 |
| `Esc`，或点击窗口外 | 关闭搜索框 |

右键托盘图标可以打开设置或退出。设置里可以开关开机启动，并录入新的全局快捷键。录入时需要带上 `Ctrl`、`Alt` 或 `Shift`。

## 从源码构建

需要：

- Windows 10 或更新版本
- [Node.js](https://nodejs.org/) 20
- [Rust](https://www.rust-lang.org/) 1.90 或更新版本
- Visual Studio 的「使用 C++ 的桌面开发」工作负载

```bash
npm install
npm run tauri dev
```

打包安装程序：

```bash
npm run tauri build
```

安装包生成在 `src-tauri/target/release/bundle/nsis/`。

## 技术栈

搜索和启动在 Rust 中完成，界面使用 Vue 3。桌面壳是 Tauri 2。

## 许可证

本项目以 [MIT 许可证](LICENSE) 发布。Copyright (c) 2026 Xiantong Li。
