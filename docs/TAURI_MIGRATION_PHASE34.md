# Tauri 迁移第 34 步：HUD 排序、窗口互斥与角色色区分度修复

## 范围

本步只修复第 33 步人工对照发现的三个回归：WebView 原生拖放的禁止光标、Settings 中
HUD 模块的双列排列、HUD 编辑器与主 DPS 面板同时可见，以及头像像素取色导致的队伍颜色
趋同。抓包、战斗归并、HUD 数据契约和现有 egui 对照入口保持不变。

## 修复结果

### HUD 与 Settings 排序

- 实际 HUD 模块和 Settings 模块顺序都改用 Pointer Events、pointer capture 和纵向命中测试，
  不再设置 HTML5 `draggable`，拖动期间不会进入 WebView 的原生禁止拖放反馈。
- 拖动只从握柄开始，目标行上半部表示插入前、下半部表示插入后，并绘制同一语义的落点线。
- Settings 始终一行一个模块；上下箭头和显示开关继续保留，仍调用既有 typed command 由 Rust
  保存唯一的 `HudConfig.module_order`。

### 主面板与 HUD 编辑器互斥

- 从 Console 打开 HUD 编辑器时，先恢复 HUD 编辑态，再隐藏 `main-dps`；隐藏主面板失败时会
  收回刚显示的 HUD，避免留下两个编辑面板。
- 从 HUD 返回主面板时，先确认主面板已显示、恢复并取得焦点，再隐藏 HUD；目标窗口操作失败时
  当前 HUD 仍可继续操作。
- Console 本身仍作为 HUD 穿透后的恢复入口保留，不参与这组主面板/HUD 的互斥关系。

### 固定高区分角色色

- 缺少显式颜色的角色改为按完整角色目录的稳定 ID 顺序分配 32 色高区分序列；角色属性、队伍
  排名和头像像素都不参与颜色选择，同属性队友也会得到不同颜色。
- 分配前保留合法的显式 `#RRGGBB`，并避开已占用颜色；当前 22 个目录角色保证颜色唯一，目录
  超过调色板容量时继续以确定性候选补足并规避精确重复。
- 主 DPS、HUD 与 egui 继续消费同一份 Rust 角色资源色；只有缺少目录记录的未知 ID 才使用
  Rust/TypeScript 一致的 32 色 FNV 回退。
- Tauri 的 `desktop` feature 不再为角色取色启用 `image`；egui `gui` 仍保留现有图片加载能力。

## 自动化回归

- Pointer 排序纯函数覆盖纵向前后落点、跳过拖动源和列表外无目标。
- Rust 覆盖 22 个同属性角色的颜色唯一性，以及不同 HashMap 插入顺序得到相同映射。
- 现有显式颜色优先级、固定未知角色回退、HUD 键盘排序、Settings 相邻移动测试继续保留。

本步已通过根 crate 的 fmt/check/test、GUI/CLI 双 Binary check/Clippy/test、CLI 依赖树检查，
以及 Tauri fmt/check/test/Clippy。前端 lint、typecheck、67 个测试文件共 227 项测试、生产构建和
Debug no-bundle Tauri 构建通过。任务文件的定向 Prettier 检查通过；整目录 `format:check` 仍只
报告 22 个本次范围外的既有工程、shadcn 和 Mod Studio 文件。

## 人工验收清单

1. **HUD 拖动**：关闭穿透，从每个模块标题条拖到另一模块上半/下半；确认没有禁止光标，落点线
   正确，释放后顺序保存；快速拖出 HUD 后释放不应误排序。
2. **Settings 拖动**：Console → Settings → HUD 模块顺序；确认任意宽度下一行一个，握柄拖动
   无禁止光标，上下箭头与开关仍可用。
3. **窗口互斥**：主 DPS 可见时从 Console 打开 HUD 编辑器，确认主 DPS 隐藏；点击 HUD 的
   主窗口按钮后确认 HUD 隐藏且主 DPS 恢复。重复切换、最小化后切换也应只有一个主编辑面板。
4. **颜色区分**：使用包含安魂曲、涔、早雾、哈尼娅的队伍，核对四条颜色明显不同；再用多个
   同属性角色组队，确认不会复用属性色。
5. **颜色一致性**：同一角色在主 DPS、HUD、占比条、百分比文字和 egui 对照入口保持一致；
   关闭并重开程序后不变。
6. **显示环境**：覆盖简中、日文、窄 Console、100%/125%/150% DPI，以及 HUD 透明、穿透、
   置顶、窗口拖动和 Console 恢复编辑状态。

本步没有启动桌面 UI；以上项目保留给人工对照验收。
