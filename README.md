# PenSwitcher

Windows 桌面 UIA 自动化工具。检查任意窗口的 UI 元素树，为元素绑定全局快捷键，一键触发。

## 功能

- **进程列表** — 枚举所有顶层窗口，支持搜索、拖拽选取
- **UIA 检查器** — 懒加载元素树，查看属性、模式支持，点击节点自动高亮
- **元素选取** — 点击目标软件定位元素，自动展开树路径
- **快捷操作** — 为任意元素绑定全局快捷键（支持多键序列如 Ctrl+1+2）
- **全局热键** — `rdev` 底层键盘钩子，离开页面后全窗口搜索定位目标
- **路径回退** — 存储完整祖先路径，节点消失时按 name/automationId 全局搜索，找不到则逐级回退激活
- **多模式激活** — Select → Invoke → Toggle → Expand → LegacyIAccessible → Click 依次尝试

## 依赖

- [Rust](https://rustup.rs/) 1.70+
- [Node.js](https://nodejs.org/) 18+
- [pnpm](https://pnpm.io/installation) (`npm i -g pnpm`)
- Windows 10+（依赖 Win32 API 和 UIAutomation COM）

## 编译

```bash
# 安装前端依赖
pnpm install

# 开发模式（热更新）
pnpm tauri dev

# 生产构建
pnpm tauri build
```

产物在 `src-tauri/target/release/`。

## 项目结构

```
src/                  # Vue 3 前端
src-tauri/src/        # Rust 后端
  main.rs             # 入口
  lib.rs              # 初始化、命令注册
  commands.rs         # 16 个 Tauri 命令
  uia.rs              # UIA 引擎（树遍历、元素激活、路径锚定）
  hotkeys.rs          # rdev 全局键盘监听、按键序列匹配
  overlay.rs          # Win32 高亮框绘制
  picker.rs           # 低层鼠标钩子（元素选取）
  windows_api.rs      # Win32 窗口枚举
  models.rs           # 数据类型（序列化）
  storage.rs          # 快捷键持久化（JSON）
  logging.rs          # 日志系统（tracing）
```

## 快捷键存储

`%LOCALAPPDATA%\PenSwitcher\shortcuts.json`

## 日志

`%LOCALAPPDATA%\PenSwitcher\logs\`
