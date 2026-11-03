# Veery

A small Windows launcher. Press Alt+Space, type, and open the app.

Veery 是一个轻量的 Windows 启动器。按 Alt+空格，在屏幕中间弹出搜索框，输入几个字，回车打开程序。

这是个人项目，更新看自己的使用需要。0.1.0 只做呼出、搜索和启动。

## 使用

程序启动后留在托盘里，窗口先藏着。

- Alt+空格呼出或隐藏
- 匹配开始菜单里的程序：全称、拼音、首字母
- 回车打开当前项，也可以点击
- Esc，或点到窗口外面，都会关掉搜索框

## 开发

需要 Windows、Node.js 20、Rust 1.90 或更新版本，以及 Visual Studio 的「使用 C++ 的桌面开发」。

```bash
npm install
npm run tauri dev
```

## 许可证

本项目以 [MIT 许可证](LICENSE) 发布。Copyright (c) 2026 Xiantong Li。
