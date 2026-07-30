# Tauri + React 迁移交接

## 当前结论

分支：`codex/tauri-react-architecture`

当前实现是 Tauri 技术验证页，不是原有战斗 HUD 的等价迁移。2026-07-30
人工验收未通过，现阶段不得替换或删除 egui HUD。

用户反馈及截图确认的问题：

1. 新窗口是技术状态面板，与原有无背板战斗 HUD 的结构、内容和视觉完全不同；
2. 无边框窗口拖动没有生效；
3. 透明背景表现没有达到原 HUD 的叠加效果；
4. WebView 内出现不均匀色块等显示异常；
5. 当前页面只展示桥接状态、Channel 序号和运行时间，没有展示团队 DPS、角色伤害、
   占比、深渊状态、迷你时间线等原 HUD 数据。

这次验收结果应视为技术验证失败，而不是可继续扩展的 HUD 产品基线。

## 已完成内容

### 工程结构

- `src-tauri/`：独立的 Tauri 2 shell，通过
  `nte-dps-tool = { path = "..", default-features = false }` 复用 Rust crate；
- `frontend/`：Vite + React + TypeScript + Tailwind CSS + shadcn/ui；
- `hud-spike`：唯一的技术验证窗口；
- typed command：读取技术快照、设置置顶、设置鼠标穿透；
- Tauri Channel：每 750 ms 推送一次有序快照，支持显式取消订阅；
- TypeScript 边界：对 Rust DTO 做运行时校验，64 位计数使用十进制字符串；
- i18n：React 继续读取 `res/languages/zh-CN.json`，没有建立第二份中文资源；
- CLI 隔离：Tauri 和 WebView 依赖没有进入 `nte-core` 的 CLI-only 依赖树；
- F12：Tauri shell 不注册全局 F12，也不依赖
  `tauri-plugin-global-shortcut`。鼠标穿透通过现有控制台入口管理。

### 当前窗口配置

`src-tauri/tauri.conf.json` 已设置：

- `transparent: true`
- `decorations: false`
- `alwaysOnTop: true`
- `shadow: false`
- `skipTaskbar: true`

`frontend/src/index.css` 也已把 `html`、`body` 和 `#root` 设为透明。
这些声明在用户机器上的最终合成结果仍未达到验收标准。

## 未通过项分析

### 1. 与原 HUD 不一致

根因明确：第一阶段实现的是独立技术状态卡片，而不是对原 HUD 的结构和数据进行迁移。

原 HUD 的事实源仍在：

- `src/app/main_view.rs` 的 `hud_panel` 及相关绘制逻辑；
- `src/app/hud.rs` 的窗口尺寸、模块布局和文本 halo；
- `src/storage/config.rs` 的 `HudConfig`、模块顺序、显隐和尺寸配置；
- `src/app/theme.rs` 的 HUD 主题 token。

下一步应先把这些行为整理成 UI-neutral 的 HUD snapshot 和配置契约，再由 React
复刻原布局。不要继续扩展当前技术仪表盘。

### 2. 窗口拖动失效

当前实现只在 React 标题区域添加了 `data-tauri-drag-region`。用户机器上未生效。

接手后按以下顺序定位：

1. 检查 `src-tauri/capabilities/default.json` 是否需要显式加入
   `core:window:allow-start-dragging`；
2. 在最小透明窗口中单独验证 drag region，不与 Card、Tooltip、按钮组合；
3. 若属性方案仍不稳定，在标题栏空白区域的主指针按下事件中调用
   `getCurrentWindow().startDragging()`；
4. 交互控件必须排除拖动，避免刷新按钮和开关吞掉或触发拖动；
5. 验证普通 DPI、125%、150% 以及多显示器。

### 3. 透明背景未达到要求

当前 React 页面使用了接近整窗尺寸的深色 Card：

```tsx
<Card className="... bg-slate-950/88 ... backdrop-blur-xl">
```

即使 WebView alpha 正常，这种结构也不是原 HUD 的“无背板直接叠加游戏画面”。此外，
用户截图中还出现了窗口区域合成不均匀的现象，需要把 CSS 绘制问题与原生窗口 alpha
问题分开定位。

推荐用最小化验证顺序：

1. 暂时只保留透明窗口和一行纯文字，移除 Card、渐变、阴影、blur 和全屏 surface；
2. 在纯色桌面背景和游戏无边框窗口上分别观察 WebView 矩形区域；
3. 确认原生透明有效后，再逐项恢复文字 halo、角色行和必要的局部半透明编辑背板；
4. 穿透展示模式不画整窗背景；编辑模式只在实际控件范围内画局部背板；
5. 若最小页面仍有矩形底色，再检查 WebView2/Tauri Windows 原生合成链路，而不是继续
   调整 Tailwind 色值。

### 4. 显示异常

用户截图可见中央区域存在不均匀深色块。当前页面同时使用透明 WebView、半透明 Card、
`backdrop-blur-xl`、多层渐变和透明 footer，变量过多。

先删除 blur、shadow 和多层渐变完成透明基线验证，再逐项恢复效果。每恢复一项都要截图
对比，定位具体触发条件。

## 推荐接手顺序

1. **保留 egui HUD**：继续作为行为和视觉基线；
2. **列出等价契约**：团队 DPS、角色行、占比、状态行、深渊半场、迷你时间线、模块顺序、
   显隐、宽度和编辑状态；
3. **最小透明壳**：只验证透明、拖动、置顶、穿透和控制台恢复入口；
4. **只读 HUD 数据**：由 Rust 投影明确 DTO，通过有序 Channel 批量推送；
5. **复刻原布局**：先完成穿透展示模式，再完成局部编辑模式；
6. **接入配置**：复用现有 `HudConfig`，不在 TypeScript 复制业务默认值；
7. **专项人工验收**：透明、拖动、穿透恢复、游戏前台置顶、多 DPI、多显示器和高负载；
8. 上述验收全部通过后，再讨论替换 egui HUD。

## 自动化验证状态

已通过：

```powershell
cargo fmt --check
cargo check
cargo test

cargo check --bin nte-dps-tool --features gui
cargo check --bin nte-core --no-default-features --features cli
cargo test --features gui
cargo test --no-default-features --features cli
cargo tree -e normal --no-default-features --features cli

cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

pnpm --dir frontend format:check
pnpm --dir frontend lint
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend build
pnpm --dir frontend tauri:build -- --debug --no-bundle
```

根项目严格 Clippy 仍会命中本分支基线已有的两处 `nonminimal_bool`：

- `src/app/mod.rs`
- `src/engine/parser.rs`

这两个文件不属于本次 Tauri 迁移改动。

## 人工复现

```powershell
pnpm --dir frontend tauri:dev
```

观察 `hud-spike`：

1. 尝试拖动顶部标题空白区域；
2. 把窗口放在具有明显纹理的背景或游戏无边框窗口之上，检查整个 WebView 矩形；
3. 开启穿透，通过现有控制台入口关闭穿透；
4. 在 100%、125%、150% DPI 下检查色块、裁切、字号和开关布局；
5. 与当前 egui HUD 的穿透展示模式逐项对比。

## 提交边界

- 本分支只增加迁移规范、Tauri shell、React 技术验证页、共享翻译键和迁移文档；
- 没有删除或替换现有 egui 入口；
- 没有修改抓包、解析、reducer、历史、更新或 Mod IPC 规则；
- 没有注册 Tauri 全局 F12；
- `frontend/dist/` 和 `src-tauri/target/` 是忽略的本地构建产物，不进入提交。
