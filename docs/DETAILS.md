# ModelHarbor 功能细节

本文为 [README](../README.md) 的补充：功能说明、各格式字段对照、配置示例、注意事项与平台安全说明。

## 技术栈

Rust（2021 edition）+ [eframe / egui](https://github.com/emilk/egui) 0.33；JSON 使用 serde_json，YAML 使用 serde_yaml_ng，文件对话框使用 rfd，网络请求使用 ureq。

## 构建运行

```bash
cargo build --release
```

产物为单文件可执行程序：`target/release/ModelHarbor.exe`

构建脚本在项目根存在 `assets/icon.png` 时调用 Python + Pillow 生成图标资源，需要 Python 与 `pillow`；不存在该源图时使用内置图标，无需 Python。

## 页面与格式

顶栏图标切换页面（opencode / Kilo Code / MiMo Code / pi / omp / DSH / ZCode / WorkBuddy）。加载任意一份配置后，各页面共享同一份数据，修改 provider 参数在所有页面同步生效（provider / model 顺序亦跨页同步）；Agents 区块仅属于 opencode 系页面。

各页表单按自身方言显示字段与枚举，**没有的字段不占位**。各页的对应关系：

| 概念 | opencode 系 | pi | oh-my-pi | DSH |
|---|---|---|---|---|
| 配置路径 | `.config/opencode/opencode.json`<br>`.config/kilo/kilo.json`<br>`.config/mimocode/mimocode.json` | `.pi/agent/models.json` | `.omp/agent/models.yml` | `.dsh/settings.yaml` |
| 协议 | `npm` | `api`（KnownApi 10 值） | `api`（官方 9 值） | `api` |
| Base URL | `options.baseURL` | `baseUrl` | `baseUrl` | `baseURL` |
| 密钥 | `options.apiKey` | `apiKey` | `apiKey`（环境变量名或字面量） | `apiKeyEnv` + `.credentials.yaml` |
| 超时 | `options.timeout` | — | — | `timeoutMs` |
| 重试 | — | — | — | `retryPolicy.mode` / `maxRetries` |
| 上下文 / 输出 | `limit.context` / `limit.output` | `contextWindow` / `maxTokens` | 同 pi | 同 pi |
| 输入模态 | `modalities.input` | `input` | `input` | `input` |
| 推理档位 | `variants`（none…ultra） | `thinkingLevelMap`（off…ultra） | `thinking.efforts` | `reasoningEfforts` |
| 模型存储 | Map（键 = model id） | Array（含 `id`） | 同 pi | 同 pi |
| Agents 区块 | 支持 | — | — | — |

协议（`npm` / `api`）各页共用同一份数据，判定顺序为：`npm` 非空按 npm 包推导 → `api` 非空直接用 → 原文件的 `api` → 都没有则按兼容层 `openai-completions`。下拉首项「(空)」表示不指定协议（`npm` 与 `api` 都清空），与 opencode 页 `npm` 的空选项同义且跨页同步。

### opencode 系：opencode / kilocode / mimocode

[Kilo Code](https://kilo.ai) 与 [MiMo Code](https://mimo.xiaomi.com/coder)（小米）都是 opencode 的 fork，配置 schema **逐字相同**——顶层 `provider` / `agent` 两个容器、`options.baseURL` / `options.apiKey` / `options.timeout`、`models.<id>.limit.context|output`、`tool_call`、`reasoning`、`modalities.input` 全部一致。因此这三页共用**同一套**解析、序列化、字段可见性与 Agents 支持实现，差别只有三项：

| | 配置目录 | 主配置文件名 | `$schema` |
|---|---|---|---|
| opencode | `.config/opencode/` | `opencode.json`（也接受 `opencode.jsonc`） | `https://opencode.ai/config.json` |
| kilocode | `.config/kilo/` | `kilo.json`（也接受 `kilo.jsonc`） | `https://app.kilo.ai/config.json` |
| mimocode | `.config/mimocode/` | `mimocode.json`（也接受 `mimocode.jsonc`） | `https://mimo.xiaomi.com/mimocode/config.json` |

因为三者内容形状完全一致，**没有任何内容特征能区分它们**，判别只能靠**路径**：命中目录名或文件名即认领对应页面，没有路径线索时才按内容（顶层 `provider` 对象）回落 opencode。后果是：把 `kilo.json` 的内容复制到 `opencode.json`，会被认作 opencode 页面——这不影响正确性，因为三者写出的 schema 完全相同，只是图标与保存路径跟着文件名走。

配置目录名以各家官方文档为准：Kilo Code 读 `~/.config/kilo/kilo.json`（另外兼容读取同目录下旧版 `opencode.json`，但不读 `.opencode/`）；MiMo Code 读 `~/.config/mimocode/mimocode.json`（同目录也接受 `config.json`），不读 `opencode.json`。

**目录名优先于文件名**：Kilo 那份遗留的 `~/.config/kilo/opencode.json` 名字像 opencode，但目录已经把它判给了 kilocode，所以归 kilocode 页面（反过来 `~/.config/opencode/kilo.json` 归 opencode）。否则按注册顺序先到的 opencode 会抢走它，页面与保存路径都会指错目录。

**MiMo Code 的页签图标不是官方 favicon。** MiMo Code 是 opencode 的 fork，它仓库里的 favicon、桌面应用图标、console logo 全都沿用 opencode 的同一份图形（`favicon.svg` 与 opencode 官方逐字节相同），照抄官方资产会得到一个和 opencode 页签**看起来一模一样**的图标。所以这里改用小米官方 logo（橙底白色 `mi`）。测试 `opencode_family_members_have_distinct_paths_and_icons` 因此按**像素差异占比**（阈值 25%）判定，而不是只比字节——同图形换抗锯齿只有约 13% 差异，换图形可达 97%。

## 获取模型

每个 provider 卡片与「新增 Provider」弹窗的 Models 标题右侧都有「获取模型」按钮，按 provider 的 api 类型请求模型列表并弹层展示：

- 展示为 checkbox 网格，列数按面板可用宽度自适应（窄窗口也不横向溢出），高度固定，超出部分在卡片内滚动；请求中显示进度指示
- 模型 id 过长时截断显示，悬停可看完整名称
- 已配置的模型自动勾选；勾选未配置的模型即新增，取消勾选不会删除已有配置
- 兼容 `data` / `models` / 裸数组三种响应结构（含 `models/` 前缀清理与去重）

## 延迟 / 连通性测试

- **厂商连通性**：Providers 标题行右侧「连通性测试」按钮，一键测试当前页面全部厂商，耗时显示在各厂商卡片名字右侧（失败显示错误码，悬停看完整错误）；卡片收起时依然可见
- **模型延迟**：每个模型行的「测试」按钮（在拖动按钮右侧，结果就在按钮右侧），**一次只测一个模型**，没有批量入口
- 测的是**首字延迟**（全程流式）：请求发出 → 第一个带内容的 SSE 块到达
- 超时判定 10 秒；着色：<2 秒绿色、2~5 秒黄色、≥5 秒红色；测试中显示动画
- 不支持自动测试的协议（`google-gemini-cli` / `bedrock-converse-stream`，需专有签名或私有网关）直接报错提示，不发无意义的请求

端点与鉴权按所选协议构造：OpenAI 兼容类打 `{base}/chat/completions`（Bearer），Responses 类打 `{base}/responses`（Azure 用 `api-key` 头），`anthropic-messages` 打 `{base}/v1/messages`（`x-api-key` + `anthropic-version`），Google 系打 `{base}/models/{model}:streamGenerateContent?alt=sse`（查询参数 `?key=`，Vertex 用 Bearer），`pi-messages` 打 `{base}/messages`。

`anthropic-messages` 的 base 两种写法都会被归一：已带 `/v1` 时补 `/messages`，未带时补 `/v1/messages`（pi / omp / DSH 的 base 不含 `/v1`，opencode 的必带 `/v1`，见「注意事项」）。

### 模型延迟的防封号保护

中转站（API 代理）普遍带「多 IP 检测 / 测活封号」风控，所以模型延迟按「单发、低频、像人」设计：

- **网络守卫**：检测到 Windows 系统代理（含 PAC）或处于 `Up` 状态的 VPN / TUN 网卡时，直接禁用模型延迟测试，并在 Providers 标题行给出原因（每 5 秒复查，关掉代理后自动恢复）；厂商「连通性测试」不受影响（只拉模型列表，不做推理）
- **放行开关**：Providers 标题行右端有「**代理支持**」勾选框（写入 `settings.json`，默认关闭）。勾上后即使检测到代理 / VPN 也允许模型延迟测试，文案转为「已放行」。本工具的探测请求**始终直连、不走系统代理**，所以仅仅开着 Clash 这类系统代理时出口 IP 没变，勾选是安全的；真正要警惕的是 **VPN / TUN 已改变出口 IP** 的情况——此时拿同一个 key 从新出口发推理探测，正是中转站风控盯的行为
- **节流**：同一厂商的任意两次探测至少间隔 5 秒，且同时只允许一个探测在飞；不同厂商之间可并行。按钮上显示剩余冷却秒数
- **问句轮换**：请求体是 24 条跨领域常识题的轮换（不校验答案，只看有没有出字与首字耗时）
- **像真实客户端**：一律流式（带 `stream: true` 与 `Accept: text/event-stream`）、测到首字后继续读到流结束才断开、`User-Agent` 按协议伪装成白名单客户端（Anthropic → `claude-cli/…`，其余 → `opencode/…`）。中转站普遍只放行白名单客户端，实测同一站点同一 key：`ureq/2.x`、不传 UA、`pi/…` 一律 `401 unauthorized client detected`
- **token 上限 16**：避免推理模型因上限过小返回空内容或整体报错
- 节流状态只在内存中，重启清零；测试仍会消耗极少量 token，不建议在计费敏感账号上频繁测试

## 查询用户数据

Providers 标题行的「查询用户数据」按钮会对当前页面的 provider **一次性取回所有能取到的账号数据**：
余额、已用、今日 / 近 7 天用量、请求次数、分组，以及**签到状态**。请求优先级为：

1. 令牌额度：`/api/usage/token/`；
2. 令牌调用日志：`/api/log/token`，用于今日、近 7 天与模型拆分；
3. 兼容账单：`/dashboard/billing/subscription` 与 `/usage`。

前两个接口**互相独立**：有些站点的 baseUrl 指向中转域名、只挂了 relay 路由，`/api/usage/token/` 会回 `Invalid URL`，但 `/api/log/token` 照样可用——此时今日 / 近 7 天与模型拆分照常显示，只是没有额度三项。两个都拿不到时才回退兼容账单。

请求只读管理接口并直连（不走系统代理）；凭证只放入请求头，不写入日志、状态栏或错误文本。同一 provider 两次查询至少间隔 5 秒。占位额度和不限额令牌只报已用、不给余额；有就显示、没有就不显示。

今日用量按本机时区从当天 0 点计算；跨过本地午夜后「今日」数字会隐藏，累计 / 余额 / 近 7 天保留，不会因此自动发起请求。日志只取首个分页，未遍历后续页时统计可能偏小。

悬停详情按两套口径分节：**账号级**（面板令牌）给余额 / 已用 / 请求次数 / 分组，**令牌级**给累计已用 / 余额 / 额度 / 今日已用 / 今日模型 / 近 7 天；末尾带两个会改变读数的提示——换算比假设（站点没给换算比时按 1 美元 = 500,000 quota 估算）与跨日提醒。

### 站点面板令牌（PAT）

上面的接口都只需 `sk-` key，但公益站的额度常是占位值（拿不到真实余额）。要拿**账号级**真实余额，在标题行点「令牌」为站点填写面板访问令牌：

- 打开的是**可拖动 / 可关闭的悬浮窗**（不占正文布局，可与右侧预览面板同时开着）；下次打开按已保存值重填
- 在哪生成：站点面板「**个人设置 → 安全设置 → 系统访问令牌**」；登录后访问 `GET /api/user/token` 也会生成一串
- 它是**站点 / 账号级**的，不是 provider 级：同一站点的多个 provider 共用一份，面板里按站点列一行并标注共用的 provider
- 站点只挂了中转路由时，面板令牌是拿到余额的**唯一途径**（账号接口 `/api/user/self` 通常仍可用）
- 与 `sk-` 是两套凭证：`sk-` 用于推理与令牌额度接口，面板令牌用于 `/api/user/self`（只读）。把 `sk-` 填进去会得到「令牌无效或已撤销」
- **用户 ID（可选）**：部署**旧版 new-api** 的站点会在 `/api/user/self` 上回 `401 New-Api-User header not provided`——令牌没问题，缺的是防跳站校验头。此时填上用户 ID（面板里 F12 看任意一次 `/api/user/self` 请求的 `New-Api-User` 值）即可；新版不需要，**留空就不发这个头**。填错会回 `does not match logged in user`；上次查询回过「缺头」的站点会把那行标黄
- 令牌无效 / 站点未开该接口 / 返回不可识别都**不影响**原有的 `sk-` 结果

## 签到状态

部分站点（new-api 的「签到」功能）每天可领一份随机额度。本工具**只读状态、不代签**——签到会改账号额度并在站点记系统日志，所以这里没有任何写入口。状态并进「查询用户数据」的结果一起显示（`GET /api/user/checkin`，需面板访问令牌与用户身份）。

展示规则是**有就输出，没有就不输出**：

- 卡片主行末尾接「签到 今日已签 $0.30」/「签到 未签」/「签到 未启用」
- 悬停详情给完整一行「签到：今日已签 $0.30（本月 14 次，累计获得 $13.00）」
- 账号接口失败但签到读到了（如缺用户 ID 被 401 的站点）→ 这一项**照样显示**
- 今日金额取自站点记录里日期最大的那条，且只在「今日已签」为真时才当今天——否则宁可不提，也不把别的日子报成今日

## 配置预览 / 编辑面板

右侧面板实时显示当前页面的待保存内容（与保存按钮同路径、同规则）：

- JSON / YAML **语法高亮**（键、字符串、数字、布尔、注释分色）
- 文本框可直接编辑：改动实时应用到左侧表单；停止输入约 0.8 秒后自动保存
- 切页 / 重新加载时重置草稿并释放焦点，不会残留上一页的文档
- 仅在编辑空闲后才按组件状态重建草稿，手改不会被冲掉；「重新生成」按钮显式放弃手改
- 格式错误时在标题行提示，编辑内容不会被写盘
- `Ctrl+F` 查找，Enter / Shift+Enter 跳转上下一个命中，Esc 关闭
- 面板左边缘的分隔条可拖动调整宽度，窗口缩放时按调整后的比例适配

## 保存与 WSL 同步

- 每页有独立保存按钮与写入路径，默认写 Windows 本地路径
- 保存按钮这一行**右端**是三个按钮：**令牌 · 显示密钥 · 预览**（分别管当前页站点的面板令牌、所有页面的密钥显隐、当前页的待保存文档）
- 当前文件属于本页格式且已加载时写当前文件；手动改了路径但未加载时写入该路径并保留目标文件其余配置
- 手动指定过的路径按页面记住（写进设置文件）：启动时优先打开「覆盖过且文件确实存在」的页面；把路径输入框清空后回车即清除该页覆盖，回到自动探测的默认路径（与默认相同的路径不会记下）
- **启动时按文件内容判定方言**，而不是只按「哪一页填了路径」：把 opencode 的配置文件填到 pi 页时，用 pi 方言去读会解析出 0 条 provider（表现为「写了路径却不自动加载」）。现在启动探测与手动加载走同一套内容判定，页面会跟随实际格式切换。文件读不出内容时保持页面推断不改判——无内容时判定会按扩展名回落 opencode，据此改判会把页面误判并污染持久化路径
- 跨格式写入由界面接管的容器：`provider`（opencode）/ `providers`（pi / omp / DSH）——容器内的条目与顺序完全来自界面，目标文件里多出来的旧条目不残留；目标文件其余顶层配置（如 `mcp`、`instructions`）原样保留
- opencode 的 `agent` 容器只在界面确实持有 agents 数据时才接管（来源为 opencode，或在 opencode 页手动新增）；来源为 pi / omp / DSH 时界面无从表达 agents，**目标文件已有的 agents 原样保留**，不会被清空
- 跨格式写入覆盖已存在的文件前，先把原内容备份为 `<文件>.bak`（内容相同或文件为空时跳过）；备份失败则取消保存，不会静默替换旧配置
- 勾选「WSL同步」后同时写入 WSL 侧对应路径；未在 WSL 中安装对应 agent 时禁用勾选

## 界面设置与状态记忆

工具的界面设置写在**家目录**的 `.modelharbor/settings.json`（Windows：`%USERPROFILE%\.modelharbor\settings.json`）。
里面只有界面选择：主题、形状、保存格式、密钥显隐、WSL 同步、卡片折叠集合、代理支持开关、各页路径覆盖。
**不存密钥、不存模型、不存配置内容**（配置永远以你自己的 agent 配置文件为真源）。

- 写入策略：内容没有变化就不写盘；有变化时先写同目录临时文件并同步，再替换正式文件；替换失败会保留原设置。读不出 / 解析失败 / 文件不存在都按默认值处理（界面设置坏了不该影响工具可用性）
- 取不到家目录时回退到 `%APPDATA%\.modelharbor\`，再回退到程序同级目录；任何情况下都不会写进 agent 配置目录
- 删除该文件即恢复默认界面设置

### 卡片折叠状态

- 记录的是**折叠**的卡片，不在记录里的就是展开：新加载的卡片默认展开
- 折叠键按“配置文件身份 / 类别 / 名字”分区；同一份配置在各页面里共享状态，不同配置文件互不覆盖
- 加载成功后只清理当前配置身份里已经不存在的记录（删掉 / 改名后不残留）；其他配置身份和加载失败场景都保留
- v2 的 `providers/名字`、`agents/名字` 旧键会在首次成功加载配置后迁移到当前配置身份

### 主题与形状

顶栏「外观」面板里两组选择，**两者正交**：主题管颜色，形状管控件长相。

**主题**（8 种，按底色分两行）：

- 深色系：深色 / 海洋 / 极地 / 苔藓
- 浅色系：亮色 / 玫瑰 / 薄荷 / 薰衣草

各自一套底色与强调色，选中后立即生效并在下次启动时恢复。状态色（绿 = 正常 / 黄 = 注意 / 红 = 异常 / 蓝 = 信息）**跨主题保持一致**，只在蓝与当前主题强调色过于接近时改用青蓝，保证在每种底色上都看得清。每个主题的正文 / 强调 / 描边都按 WCAG 下限校过（正文与强调 4.5:1、描边 3:1），提示色另有下限（3:1）且必须明显淡于正文。

**形状**（8 种，4+4 两行）：

| 形状 | 机制 |
|---|---|
| 标签 | 胶囊全圆角 + 0.5px 细边 |
| 圆润 | 圆角 10 + 1px 描边（默认档） |
| 精致 | 圆角 6 + 0.5px 细边，轻量感 |
| 极简 | 全直角 + 无边框，纯色块 |
| 云朵 | 大圆角 16 + 软投影，卡片浮在底上 |
| 浮雕 | 凸起受光线（卡内左上亮 / 右下暗）+ 接触影 |
| 石板 | 平放的板——深色描边 + 接触影，无受光线 |
| 色带 | 卡片顶部 3px 主题强调色条 |

每档是**不同的绘制机制**，而不是同一效果调强度。浅色主题下浮雕 / 石板的暗边会加重到近实色——白底上白高光隐形，立体感全靠暗边，不加重就会退化成普通卡片。

无边框的形状（极简 / 云朵）悬停时会在卡片内侧浮现 1px 高亮环；高亮色按主题取（深色系用强调色，浅色系用中性深灰，避免浅底上出现突兀的饱和色环）。

旧设置里已删除的形状键（`heavy` / `sharp` / `compact` / `neon` / `frosted` / `prism`）会自动回落到默认档（圆润），不需要手工清理 `settings.json`。

### 旧版本位置兼容

早期版本把设置放在 `%APPDATA%\.modelharbor\prefs.json`，后来移到家目录且文件名改为 `settings.json`。
读取时按「家目录 settings.json → 家目录 prefs.json → `%APPDATA%` 下两者」依次回退，
写盘只写新位置的新名字，并在首次保存成功后清掉家目录同目录的旧文件（老设置不会丢）。

## 缺省值与字段映射

- 配置未写 `timeout` / `timeoutMs` 时显示默认 `180000` ms，未修改时不写回
- DSH 的 `retryPolicy.mode` 缺省显示 `normal`
- pi 的 `compat.requiresReasoningContentOnAssistantMessages` 与 omp 的 `compat.requiresReasoningContentForAllAssistantTurns` 相互映射；加载 opencode / DSH 或新建时默认不勾选

## 配置文件格式参考

字段细节由界面表单呈现，这里只说明各格式的**结构形状**（加新 agent 时按同一张表扩展）。

**JSON 系**——opencode 系三页的 `provider` 与 `models` 都是 **Map**（键 = 名字 / model id），schema 相同，此处以 opencode 为例；pi 的 `providers` 是 Map、`models` 是 **Array**（每项含 `id`）：

```jsonc
// opencode.json —— 顶层另有 agent 容器
{
  "agent":    { "<agent 名>": { "mode": "subagent", "model": "…", "system": "…" } },
  "provider": { "<provider key>": {
      "npm": "@ai-sdk/openai",
      "options": { "baseURL": "…", "apiKey": "…", "timeout": 180000 },
      "models": { "<model id>": { "name": "…", "limit": { "context": 0, "output": 0 } } }
  } }
}

// ~/.pi/agent/models.json —— 无 agent 容器
{
  "providers": { "<provider key>": {
      "baseUrl": "…", "apiKey": "…", "api": "openai-completions",
      "models": [ { "id": "…", "name": "…", "contextWindow": 0, "maxTokens": 0 } ]
  } }
}
```

**YAML 系**——omp 与 pi 同族（推理档位存为 `thinking` 块）；DSH 只管理 `llm-pi-ai.providers`，其余顶层配置原样保留，密钥是 `apiKeyEnv` 引用（实际值在同级 `.credentials.yaml`）：

```yaml
# ~/.omp/agent/models.yml
providers:
  <provider key>:
    baseUrl: …
    api: openai-completions
    apiKey: …          # 环境变量名或字面量
    models:
    - id: …
      thinking: { mode: effort, efforts: [medium, high] }

# ~/.dsh/settings.yaml
llm-pi-ai:
  providers:
    <provider key>:
      apiKeyEnv: MY_API_KEY
      api: openai-completions
      baseURL: …
      retryPolicy: { mode: normal, maxRetries: 3 }
      models:
        - id: …
          reasoningEfforts: { medium: medium }
```

## 注意事项

- `baseURL` 末尾 `/v1` 的归一化按目标 agent 的客户端行为决定，**读入与写出都做**：pi / omp / DSH 的 `anthropic-messages` **去掉**末尾 `/v1`（这三家客户端自己拼 `/v1/messages`，base 里再带会请求成 `/v1/v1/messages`）；opencode 的 `@ai-sdk/anthropic` 相反，baseURL **必须带** `/v1`（客户端只追加 `/messages`）。其他 api 一律不动
- provider / model 只保存各自支持的字段，方言字段不会互相泄漏
- omp 的 `apiKey` 为「环境变量名或字面量」语义；推理档位保存为官方 `thinking` 块
- 保存 YAML 时文件注释不会保留，输出为标准块风格
- DSH 的实际密钥保存在同级 `.credentials.yaml`，加载时自动读取、保存时写回；凭据文件中的其他字段原样保留

## 平台与安全

- 当前**仅支持 Windows**
- 配置文件中的 `apiKey` 为**明文**（DSH 的 `.credentials.yaml` 同样），请勿提交到公开仓库
- 界面设置文件只保存界面选择、不含密钥，可以安全删除（会恢复默认界面设置）
- 站点面板令牌存在 `%USERPROFILE%\.modelharbor\tokens.json`，**含凭证且为明文**（与 agent 配置文件同级风险），请勿提交或同步到共享目录。里面只有你主动填过的站点，在「令牌」面板点「删除」或直接删除该文件即可清空
- 令牌与用户 ID **不会**写进 `settings.json`，也不会写进任何 agent 配置文件；接口请求只把它们放进请求头（不进 URL、不进日志与状态栏文本）
