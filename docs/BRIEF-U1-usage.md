# BRIEF U-1：本机用量统计（tokscale-core 扫描 + 四维统计 UI + 账本）

## 目标

在 ModelHarbor 顶栏加一个按钮，打开「本机用量」悬浮窗：按 **agent / 模型 / 会话 / 时间** 四个维度展示本机 10 个 AI 编程 agent 的 token 使用量。每 3 秒自动刷新（增量检测），启动时自动追加更新，已删除的会话用量永久保留。

---

## 一、已完成的调研（**不要重复验证**）

### 1.1 技术选型：已定 `tokscale-core`

**依赖**（已实测可编译、可运行）：

```toml
[dependencies]
tokscale-core = { git = "https://github.com/junhoyeo/tokscale", tag = "v4.18.0" }
```

- 上游：`junhoyeo/tokscale`（5,649 stars，**MIT**，纯 Rust workspace）
- 它是 `crates/tokscale-core`（rlib 库）+ `crates/tokscale-cli`（CLI）
- 未发布到 crates.io → **只能用 git 依赖**，已钉 tag `v4.18.0`（commit `d4d1c75185`）
- 原生支持 **56 个 client**，覆盖 ModelHarbor 全部 10 个 agent

### 1.2 唯一入口 API

```rust
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::{parse_local_clients, LocalParseOptions};

let opts = LocalParseOptions {
    home_dir: None,          // None = 用当前用户 home
    use_env_roots: false,
    clients: Some(vec![...]),// 只传 10 个（见 1.3）
    since: None, until: None, year: None,
    scanner_settings: ScannerSettings::default(),
};
let parsed: ParsedMessages = parse_local_clients(opts)?;  // Result<_, String>
```

`ParsedMessages { messages: Vec<ParsedMessage>, counts: ClientCounts, processing_time_ms: u32 }`

`ParsedMessage` 字段（**四维统计全在这里**）：

```rust
pub struct ParsedMessage {
    pub client: String,          // agent（tokscale id，需反查回 ConfigFormat）
    pub model_id: String,        // 模型
    pub provider_id: String,
    pub session_id: String,      // 会话
    pub workspace_key: Option<String>,
    pub workspace_label: Option<String>,
    pub timestamp: i64,          // 毫秒
    pub date: String,            // "YYYY-MM-DD"
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cache_write_1h: i64,     // cache_write 的子集，不要重复加
    pub reasoning: i64,
    pub duration_ms: Option<i64>,
    pub message_count: i32,
    pub agent: Option<String>,
    pub cost: f64,
    pub cost_source: CostSource,
    pub service_tier: Option<String>,
}
```

### 1.3 agent id 映射表（**6/10 不一致，必须显式映射**）

| ModelHarbor `ConfigFormat` | tokscale client id |
|---|---|
| `Opencode` | `opencode` |
| `Kilocode` | `kilo` |
| `Mimocode` | `micode` |
| `Pi` | `pi` |
| `OhMyPi` | `omp` |
| `DeepSeekHarness` | `dsh` |
| `ZCode` | `zcode` |
| `WorkBuddy` | `workbuddy` |
| `QwenCode` | `qwen` |
| `KimiCode` | `kimi` |

⚠️ **`kilo` vs `kilocode` 的坑**：tokscale 有两个 Kilo 相关 id。
- `kilo` = **Kilo CLI**，`~/.local/share/kilo/kilo.db`（SQLite）→ **ModelHarbor 的 `Kilocode` 对应这个**
- `kilocode` = VS Code **扩展**，`~/.config/Code/User/globalStorage/kilocode.kilo-code/tasks/ui_messages.json`

⚠️ **`micode` 拼写**：ModelHarbor 叫 `mimocode`，tokscale 叫 `micode`。

### 1.4 实测性能数据（真机，release）

| 项 | 数值 |
|---|---|
| 全量扫描（10 个 client） | **2.4-2.7s** |
| 空闲时 stat 全部源文件（43 个） | **0.76ms** |
| 首次 release 编译 | 3m29s |
| **增量编译（改叶子 crate）** | **21s**（需 `lto="thin"`，见 1.6） |

⚠️ **`parse_local_clients` 内部无缓存**，每次调用都是全量扫描（2.4s）。**不能**每 3 秒调一次 → 必须分层（见三、架构）。

### 1.5 数据正确性（已实测，勿重算）

```
opencode   in=86,392,970  out=2,641,782  cache_r=1,227,947,072  cache_w=4,459,724
pi         in=80,065,028  out=4,223,274  cache_r=  735,954,944  cache_w=1,411,975
workbuddy  in=14,313,601  out=3,265,383  cache_r=  633,630,267
dsh        in=    29,051  out=   14,680  cache_r=      437,248
共 19,179 条消息
```

**口径说明**：tokscale 的 `input` 是**新鲜输入**（已排除 cache_read），不要把 cache_read 加进 input。`cache_write_1h` 是 `cache_write` 的子集，**不要重复加**。

### 1.6 ⚠️ Cargo profile 必须调整（否则增量编译 2m15s）

ModelHarbor 现有 profile：
```toml
[profile.release]
opt-level = "z"
lto = true          # ← fat LTO，与重依赖树结合后增量编译 2m15s
codegen-units = 1   # ← 同样拖慢
panic = "abort"
strip = true
```

**实测对比**（改叶子 crate 的增量编译）：

| 配置 | 增量编译 | 扫描耗时 | exe |
|---|---|---|---|
| 现状（`lto=true`, `cgu=1`） | **2m15s** ❌ | 3.0s | 4.2MB |
| `lto="thin"`, `cgu=16` | **21s** ✅ | 2.5s | 5.2MB |

**必须改成**：
```toml
[profile.release]
opt-level = "z"
lto = "thin"          # fat → thin
codegen-units = 16    # 1 → 16
panic = "abort"
strip = true

# 给 tokscale-core 单独提速：它吃 opt-level="z" 会慢 20%
[profile.release.package.tokscale-core]
opt-level = 3
codegen-units = 16
```

**已验证**：tokscale-core 里 5 处 `catch_unwind` **全在 `#[cfg(test)]` 内**，与 `panic = "abort"` 不冲突。

---

## 二、需求

1. **顶栏加按钮** → 打开「本机用量」悬浮窗（照 `ui_health_window` / `ui_tokens_window` 的模式）
2. **四维统计**：按 agent / 模型 / 会话 / 时间
3. **每 3 秒自动刷新**（增量检测，空闲时近零开销）
4. **启动时自动追加更新**（打开软件即扫一次）
5. **已删除的会话用量永久保留**（账本机制，见三.3）
6. 配色自己定（用现有 `crate::theme::semantics(ui)` 的语义色）

---

## 三、架构

### 3.1 分层刷新（**关键设计**）

```
每 3s（UI 帧驱动）：
  ├─ stat 全部源文件（43 个 ≈ 0.76ms）
  ├─ 比对 mtime + size 指纹
  └─ 无变化 → 直接用上次结果，零开销返回
       │
       └─ 有变化 → debounce 1.5s（上限 5s）→ 后台线程调 parse_local_clients（2.4s）
```

**debounce 参数**（照 token-monitor 的实测值）：
- `debounce = 1500ms`：连续变化合并成一次
- `max_wait = 5000ms`：防止持续写入无限推迟（agent 流式输出会持续改文件）
- **无额外冷却**（token-monitor 的注释明确说「There is deliberately no cooldown on top of the debounce」）

### 3.2 后台线程 + mpsc（照 `app/balance.rs` 的现成模式）

```rust
// 状态字段（加进 App struct）
usage: UsageState,

pub(super) struct UsageState {
    result: Option<Result<UsageReport, String>>,
    rx: Option<Receiver<Result<UsageReport, String>>>,
    last_scan_at: Option<f64>,       // egui 时间轴秒
    fingerprints: HashMap<PathBuf, (u64, u64)>,  // path → (mtime, size)
    dirty_since: Option<f64>,        // debounce 起点
    last_full_scan: Option<f64>,     // 用于 max_wait
}
```

在 `update()` 里加 `self.poll_usage();`（和 `poll_balance()` 并列）。

### 3.3 账本机制（保留已删会话）—— 照 token-monitor 的 `sessionUsageArchive`

**问题**：opencode/workbuddy 的会话被删后，其用量从源里消失。
**方案**：ModelHarbor 自建一份账本，把每次扫描到的会话快照存下来，展示时 replay 回去。

```
每次扫描后：
  对每个 session_id（key = "client:session_id"）：
    - 若账本无此条 → 新增
    - 若有 → 用最新值覆盖（token 只会增长）
    - 记录 capturedAt

展示时：
  实时扫描结果 ∪ 账本中已消失的会话
  已消失的标记 archived: true（UI 上可加个「已删除」小标记）
```

**存储位置**：`%APPDATA%\ModelHarbor\usage-ledger.json`（用户已确认用 ModelHarbor 自己的配置目录）

**存储格式**（简化版，够用即可）：
```json
{
  "version": 1,
  "sessions": {
    "opencode:ses_abc": {
      "client": "opencode",
      "session_id": "ses_abc",
      "model_id": "claude-opus-5",
      "captured_at": "2026-10-10T11:00:00Z",
      "input": 123, "output": 456, "cache_read": 789, "cache_write": 0, "reasoning": 0,
      "first_seen": "...", "last_seen": "..."
    }
  }
}
```

**注意**：
- **原子写**（写临时文件 + rename），避免崩溃时损坏
- 只写**变化的** key（差异写），不是每次全量重写
- 读失败/损坏 → 静默重建，不能让账本损坏导致功能不可用

### 3.4 模块划分

```
crates/core/src/usage/
  mod.rs          -- 公开 API：scan()、UsageReport、UsageEntry
  agents.rs       -- ConfigFormat ↔ tokscale client id 映射表（见 1.3）
  ledger.rs       -- 账本读写（原子写、差异写、损坏恢复）
  watch.rs        -- 指纹（mtime+size）+ debounce 判定（纯逻辑，可单测）

crates/app/src/app/
  usage.rs        -- UsageState + poll_usage + 后台线程（照 balance.rs）
  windows.rs      -- 加 ui_usage_window（照 ui_health_window）
  bars/top_bar.rs -- 加按钮
```

**分层原则**：`core/src/usage/` 是**纯逻辑**（不依赖 egui），`app/src/app/usage.rs` 只管线程与 UI 状态。这样 core 部分能单测覆盖。

---

## 四、验收标准

### 4.1 三件套（必须全过）

```bash
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### 4.2 必须有的单测（core/src/usage/）

- `agents.rs`：10 个 `ConfigFormat` 全部能映射到 tokscale id（含 `Kilocode→kilo`、`Mimocode→micode` 这两个易错项）
- `ledger.rs`：
  - 新增会话 → 写入
  - 已有会话 token 增长 → 覆盖
  - 会话从源里消失 → 账本仍保留（**核心需求**）
  - 账本文件损坏 → 静默重建，不 panic
  - 原子写：写入过程中断不产生半个文件
- `watch.rs`：
  - 无变化 → 判定为「不需要重扫」
  - mtime 变化 → 判定为「需要重扫」
  - size 变化 → 同上
  - debounce：1.5s 内的多次变化合并成一次
  - max_wait：持续变化超过 5s 强制触发

### 4.3 手工验证

1. 打开软件 → 用量窗口自动有数据（不用手点刷新）
2. 用 pi/opencode 跑一轮对话 → 3-5 秒内窗口数字上涨
3. 窗口空闲时 CPU 占用接近 0（用任务管理器看）
4. 关掉软件重开 → 数字不减少（账本生效）
5. 四维切换：按 agent / 模型 / 会话 / 时间都能出数

---

## 五、约束与红线

### 5.1 绝对禁止

- ❌ **不要用 `find` 命令**（多个 worker 曾因 `find ~` 卡死超时）
- ❌ **不要查找 skills / 计划文档 / 历史 brief**（浪费大量时间）
- ❌ 不要改动 `crates/core/src/backends/` 里的现有后端逻辑
- ❌ 不要引入除 `tokscale-core` 之外的**新**依赖（tokscale 自带的传递依赖除外）
- ❌ 不要动 `panic = "abort"`（已验证兼容）
- ❌ 不要在 UI 线程里调 `parse_local_clients`（2.4s 会卡死界面）

### 5.2 必须遵守

- ✅ 改 `Cargo.toml` 的 profile（见 1.6），这是硬性要求
- ✅ 所有扫描在后台线程，UI 只读结果（照 `balance.rs`）
- ✅ 账本写盘用原子写（临时文件 + rename）
- ✅ 中文注释，与现有代码风格一致（看 `balance.rs` 的注释风格）
- ✅ 复用现有主题语义色 `crate::theme::semantics(ui)`，不要硬编码颜色
- ✅ 悬浮窗照 `ui_health_window` 的写法（`.order(Self::TOKENS_WINDOW_ORDER)`、`.constrain_to(area)`、`floating_window_frame`）

### 5.3 首次编译会很慢

加 `tokscale-core` 后**首次** `cargo build --release` 约 3.5 分钟（要编 165+ 个 crate 含 LTO）。这是正常的，不是卡死。之后的增量编译 21s。

---

## 六、UI 建议（配色自定，仅供参考）

**悬浮窗结构**：

```
┌─ 本机用量 ────────────────────────────┐
│ [Agent] [模型] [会话] [时间]   ↻ 12:34:56 │  ← 维度切换 tab + 上次刷新时间
├───────────────────────────────────────┤
│ 汇总：19,179 条消息 · 10 个 agent        │
│                                       │
│ ┌─ pi ──────────────────────────────┐ │
│ │ 输入 80.1M  输出 4.2M  缓存读 736M │ │
│ │ 18 会话 · 15 模型 · 最近 2 分钟前   │ │
│ └───────────────────────────────────┘ │
│ ┌─ opencode ────────────────────────┐ │
│ │ 输入 86.4M  输出 2.6M  缓存读 1.23B│ │
│ │ 21 会话 · 21 模型 · 最近 5 分钟前   │ │
│ └───────────────────────────────────┘ │
│ ...                                   │
└───────────────────────────────────────┘
```

**数字格式化**：大数用 `K` / `M` / `B` 后缀（tokscale 自带 `compactTokens` 逻辑可参考，或自己写）。

**已删除会话**：在会话维度里加个小标记（如灰色的「已删除」或 `archived` 图标），用 `semantics.warn` 或弱化色。

---

## 七、交付物

1. `crates/core/src/usage/` 新模块（含单测）
2. `crates/app/src/app/usage.rs` + `windows.rs` 加窗口 + `bars/top_bar.rs` 加按钮
3. `Cargo.toml` profile 调整
4. 提交信息：`feat(usage): local token usage stats via tokscale-core`
