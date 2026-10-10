# BRIEF U-1b：本机用量 UI（悬浮窗 + 四维统计 + 3 秒增量刷新）

## 前置：地基已完成并提交

上一阶段已提交 `4efad41`，**这些都已验证可用，直接用，不要重写**：

### `model_harbor_core::usage` 模块（`crates/core/src/usage/`）

```rust
// 扫描本机全部 10 个 agent 的本地会话账本。
// ⚠️ 全量扫描，实测 release 2.4-2.7s / debug 14.6s —— 必须放后台线程。
pub fn scan() -> Result<ScanResult, String>;

pub struct ScanResult {
    pub sessions: HashMap<String, SessionSnapshot>,  // key = "client:session_id"
    pub message_count: usize,
    pub processing_time_ms: u32,
}

// 账本：让已删除会话的用量永久保留
pub struct Ledger { /* ... */ }
impl Ledger {
    pub const FILE_NAME: &'static str = "usage-ledger.json";
    pub fn default_path() -> PathBuf;        // ~/.modelharbor/usage-ledger.json
    pub fn open_default() -> Self;
    pub fn open(path: impl Into<PathBuf>) -> Self;
    pub fn in_memory() -> Self;              // 测试用
    pub fn merge_scan(&mut self, scanned: HashMap<String, SessionSnapshot>);
    pub fn view(&self) -> Vec<SessionSnapshot>;   // 实时 ∪ 已归档，按最近活动倒序
    pub fn save(&mut self) -> std::io::Result<()>; // 差异写 + 原子写，无改动不产生 IO
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_dirty(&self) -> bool;
}

pub struct SessionSnapshot {
    pub client: ConfigFormat,
    pub session_id: String,
    pub model_id: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub message_count: i64,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub archived: bool,        // true = 已从源里消失，用量由账本保留
}
impl SessionSnapshot {
    pub fn key(client: ConfigFormat, session_id: &str) -> String;
    pub fn total(&self) -> i64;     // input+output+cache_read+cache_write+reasoning
    pub fn has_usage(&self) -> bool;
}
```

**已通过**：24 个单测 + 真机扫描（46 会话 / 19,237 消息 / 4 个 agent 有数据）。

---

## 任务：只改 app crate

### 1. `crates/app/src/app/usage.rs`（新建）—— 后台线程 + 增量刷新

**照 `crates/app/src/app/balance.rs` 的现成模式**（后台线程 + `mpsc::Receiver` + `try_recv`），先读它。

```rust
pub(super) struct UsageState {
    /// 最近一次成功扫描的结果（已并入账本）。
    result: Option<Result<UsageReport, String>>,
    rx: Option<Receiver<Result<UsageReport, String>>>,
    /// 账本（跨扫描保留已删会话）。
    ledger: Ledger,
    /// 上次扫描时刻（egui 时间轴秒）。
    last_scan_at: Option<f64>,
    /// 上次落盘时刻。
    last_save_at: Option<f64>,
    /// 源文件指纹：path → (mtime_secs, size)。
    fingerprints: HashMap<PathBuf, (u64, u64)>,
    /// debounce：首次检测到变化的时刻。
    dirty_since: Option<f64>,
}
```

**核心逻辑**：

```rust
// 在 App::update() 里调用（和 poll_balance() 并列）
pub(super) fn poll_usage(&mut self) {
    // 1. 收结果（照 balance.rs 的 try_recv 模式）
    //    Ok(report) → 存进 result，写账本，rx = None
    //    Empty → 什么都不做
    //    Disconnected → rx = None
    //
    // 2. 若 rx.is_some() → 扫描在途，直接返回（不要叠加扫描）
    //
    // 3. 变更检测（每 3 秒一次，用 last_scan_at 节流）
    //    let now = ui.input(|i| i.time);  // 或 ctx.input
    //    if 距上次检测 < 3.0s → return
    //    比对指纹：对每个源路径 stat，mtime/size 变了就是脏
    //
    // 4. debounce 判定：
    //    - 首次发现变化 → dirty_since = Some(now)，先不扫
    //    - 变化持续：now - dirty_since >= 1.5s → 触发扫描
    //    - 兜底：now - dirty_since >= 5.0s → 强制触发（agent 流式写入会持续改文件）
    //
    // 5. 触发：spawn 后台线程调 model_harbor_core::usage::scan()
}
```

**debounce 参数（照 token-monitor 实测值，不要改）**：
- `DEBOUNCE_SECS = 1.5`：连续变化合并成一次
- `MAX_WAIT_SECS = 5.0`：持续写入时的强制触发上限
- `POLL_INTERVAL_SECS = 3.0`：指纹检测间隔
- **无额外冷却**（token-monitor 明确注释「There is deliberately no cooldown on top of the debounce」）

**源文件指纹怎么取**：tokscale 的源路径因 agent 而异，**不要自己拼路径**。最简单可靠的做法是：
- 记录扫描开始/结束时刻，用「上次扫描完成时间」和「下次检测时间」做节流即可；或者
- 对已知的几个根目录做**目录 mtime** 比对（`~/.pi/agent/sessions`、`~/.local/share/opencode`、`~/.workbuddy/projects`、`~/.dsh/sessions`）——目录 mtime 在子文件新增/删除时会变，但**文件内容追加不会改目录 mtime**。
- ⚠️ 因此推荐：**对具体文件 stat**。可以先做一次扫描时记录所有 `.jsonl` / `.db` 文件的 mtime+size（用 `std::fs::read_dir` 递归，**不要用 `find` 命令**），后续只 stat 这些已知路径。

**若指纹实现复杂，降级方案**（可接受）：固定每 3 秒轮询一次「源目录里最新文件的 mtime」，无变化则跳过扫描。**关键是不能每 3 秒全量扫描**（2.4s 会占满 CPU）。

**报告结构**（`UsageReport`，自己定义）：

```rust
pub(super) struct UsageReport {
    pub sessions: Vec<SessionSnapshot>,      // 已按最近活动倒序
    pub scanned_at: f64,
    pub message_count: usize,
    pub processing_time_ms: u32,
}
```

### 2. `crates/app/src/app/windows.rs` —— 加悬浮窗

**照同文件里的 `ui_health_window` 写法**（`.order(Self::TOKENS_WINDOW_ORDER)`、`.fixed_size(size)`、`.constrain_to(area)`、`.current_pos(centered)`、`self.floating_window_frame(ctx)`、`ScrollArea`）。

窗口标题「本机用量」。尺寸建议 `egui::vec2(760.0, height)`（比体检窗宽一点，要放表格）。

### 3. 四维统计视图

窗口内用一组**维度切换按钮**（Agent / 模型 / 会话 / 时间），当前维度高亮（复用顶栏页签的选中样式：`ui.visuals().widgets.hovered.bg_fill` + 2px 描边，见 `bars/top_bar.rs`）。

**四个维度的聚合**（都在 app 层从 `Vec<SessionSnapshot>` 现算，不要改 core）：

| 维度 | 聚合方式 | 每行显示 |
|---|---|---|
| **Agent** | 按 `snapshot.client` 分组 | agent 名（`format.label()`）、输入/输出/缓存读、会话数、最近活动 |
| **模型** | 按 `snapshot.model_id` 分组 | 模型名、输入/输出/缓存读、会话数、用到它的 agent |
| **会话** | 每条一个（已排好序） | 会话 id（截断显示）、agent、模型、合计 token、最近活动；`archived` 的加标记 |
| **时间** | 按 `last_seen_ms` 的**本地日期**分组 | 日期、当天合计 token、当天会话数 |

**时间维度的日期换算**：用 `chrono`？**不行，core 没有 chrono 依赖**。用 `std::time::SystemTime` 手算本地日期，或参考项目里已有的做法（`crates/core/src/billing.rs` 里有「今日 0 点」的本地时区处理，用 `windows-sys` 的 `GetLocalTime`）。**直接复用那个**。

**数字格式化**：写一个 `format_tokens(n: i64) -> String`：
- `< 1_000` → 原样
- `< 1_000_000` → `12.3K`
- `< 1_000_000_000` → `45.6M`
- 否则 → `1.23B`

**相对时间**：`last_seen_ms` → 「2 分钟前」/「3 小时前」/「5 天前」。

### 4. `crates/app/src/app/bars/top_bar.rs` —— 加按钮

在工具按钮区（`show_tokens` / `show_health` 那一排，约 540-620 行）加一个按钮：

```rust
// 用量：本机各 agent 的 token 使用量（独立悬浮窗）。
let usage_btn = toolbar_icon_button(
    ui,
    self.toolbar_icons.<合适的图标>.as_ref(),
    self.show_usage,
    ui.visuals().text_color(),
)
.on_hover_text("本机用量");
if usage_btn.clicked() {
    self.show_usage = !self.show_usage;
}
```

**图标**：看 `crates/app/assets/icons/` 里现有哪些可复用（`activity.svg` 已被体检用了，`database.svg` 已被别的用了）。可以复用 `database` 或找一个语义接近的。**不要新增 SVG 文件**（避免引入新的资源依赖问题）；若实在没有合适的，用 `activity` 也行（两个窗口不会同时开）。

### 5. `crates/app/src/app/mod.rs` —— 状态字段 + 接线

```rust
// App struct 里加（放在 show_health 附近）
/// 本机用量悬浮窗是否打开。
show_usage: bool,
/// 本机用量扫描状态（见 [`crate::app::usage`]）。
usage: usage::UsageState,
```

- 初始化：`show_usage: false`，`usage: UsageState::default()`（或 `UsageState::new()`）
- `update()` 里加 `self.poll_usage();`（在 `poll_balance()` 旁边）
- 末尾加 `self.ui_usage_window(ctx);`（在 `ui_health_window(ctx)` 旁边）

**启动时自动扫描**：`UsageState` 首次 `poll_usage` 时 `result.is_none() && rx.is_none()` → 立即触发一次扫描（不等 3 秒），满足「每次打开软件自动追加更新」。

---

## 验收标准

### 三件套（必须全过）

```bash
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### 必须有的测试（app 层，纯逻辑部分）

把「聚合 + 格式化」做成**可单测的纯函数**，至少覆盖：

- `format_tokens`：999 / 1000 / 999_999 / 1_000_000 / 1_234_567_890 的边界
- 四维聚合：给定一组 `SessionSnapshot`，各维度的分组结果正确
- `archived` 会话在会话维度里被标记
- 相对时间：刚刚 / 分钟 / 小时 / 天 的边界

（egui 绘制代码不要求单测，但纯逻辑部分要有。）

### 手工验证

1. 打开软件 → 点用量按钮 → 窗口有数据（不用手点刷新）
2. 窗口里四个维度都能切换出数
3. 用 pi 或 opencode 跑一轮对话 → 3-5 秒内数字上涨
4. **窗口开着时任务管理器里 CPU 占用接近 0**（空闲时不扫描）
5. 关掉软件重开 → 数字不减少

---

## 约束与红线

### 绝对禁止

- ❌ **不要用 `find` 命令**（曾导致 worker 卡死超时）
- ❌ **不要查找 skills / 计划文档 / 历史 brief**
- ❌ 不要改 `crates/core/src/usage/` 里已提交并验证的代码（地基）
- ❌ 不要改 `crates/core/src/backends/` 里的后端逻辑
- ❌ 不要引入新依赖
- ❌ **不要在 UI 线程调 `usage::scan()`**（debug 14.6s 会卡死界面）
- ❌ 不要每 3 秒全量扫描（必须增量检测）
- ❌ 不要硬编码颜色（用 `crate::theme::semantics(ui)`）

### 必须遵守

- ✅ 中文注释，风格照 `balance.rs`（讲清「为什么」，不只讲「做什么」）
- ✅ 悬浮窗照 `ui_health_window` 的写法
- ✅ 扫描在后台线程，UI 只读结果
- ✅ 复用 `crate::prefs::Prefs::config_dir()` 取账本目录（不要自己拼路径）
- ✅ 首次编译慢（tokscale 依赖树），这是正常的

### 参考文件（先读这些）

1. `crates/app/src/app/balance.rs` —— 后台线程 + mpsc 的**标准模式**
2. `crates/app/src/app/windows.rs` —— `ui_health_window` 悬浮窗写法
3. `crates/app/src/app/bars/top_bar.rs` 540-620 行 —— 工具按钮区
4. `crates/core/src/usage/` —— 已完成的扫描 + 账本 API
5. `crates/core/src/billing.rs` —— 本地日期换算的现成做法

---

## 交付物

1. `crates/app/src/app/usage.rs`（新建）
2. `crates/app/src/app/windows.rs`（加 `ui_usage_window`）
3. `crates/app/src/app/bars/top_bar.rs`（加按钮）
4. `crates/app/src/app/mod.rs`（状态字段 + 接线）
5. 提交信息：`feat(usage): 本机用量悬浮窗（四维统计 + 3s 增量刷新）`
