# ModelHarbor 功能细节

本文为 [README](../README.md) 的补充：各页面能做什么、各格式字段对照、配置示例、注意事项与平台安全说明。

## 技术栈

Rust（2021 edition）+ [eframe / egui](https://github.com/emilk/egui) 0.33；JSON 使用 serde_json，YAML 使用 serde_yaml_ng，文件对话框使用 rfd，网络请求使用 ureq。

## 构建运行

```bash
cargo build --release
```

产物为单文件可执行程序：`target/release/ModelHarbor.exe`

构建脚本在项目根存在 `assets/icon.png` 时调用 Python + Pillow 生成图标资源，需要 Python 与 `pillow`；不存在该源图时使用内置图标，无需 Python。

## 页面与格式

顶栏图标切换页面，共 9 个后端：

| 页面 | 配置路径 |
|---|---|
| opencode | `.config/opencode/opencode.json` |
| Kilo Code | `.config/kilo/kilo.json` |
| MiMo Code | `.config/mimocode/mimocode.json` |
| pi | `.pi/agent/models.json` |
| oh-my-pi | `.omp/agent/models.yml` |
| DeepSeek Harness | `.dsh/settings.yaml` |
| ZCode | `.zcode/v2/provider_config.json` |
| WorkBuddy | `.workbuddy/models.json` |
| Qwen Code | `.qwen/settings.json` |

以上是各页默认探测的路径。**JSON 配置一律接受 `.jsonc`**（含注释的 JSON）；默认路径不存在时，opencode 系三页还会自动找同名的 `.jsonc`。

加载任意一份配置后，各页面共享同一份数据，修改 provider 参数在所有页面同步生效（provider / model 顺序亦跨页同步）；Agents 区块仅属于 opencode 系页面。

**Kilo Code 与 MiMo Code 都是 opencode 的 fork**，schema 的字段与结构一致（顶层 `provider` / `agent`、`options.baseURL`、`models.<id>.limit.context|output`），因此三页共用同一套表单与编辑逻辑，差别只有配置目录名、主配置文件名与图标。三者内容形状一致，**判别按配置路径**（目录名 / 文件名），没有路径线索时回落 opencode；目录名优先于文件名（`~/.config/kilo/opencode.json` 归 Kilo Code 页）。

各页表单按自身方言显示字段与枚举，**没有的字段不占位**。各页的对应关系：

| 概念 | opencode 系 | pi | oh-my-pi | DSH | ZCode | WorkBuddy | Qwen Code |
|---|---|---|---|---|---|---|---|
| 协议 | `npm` | `api`（10 值） | `api`（9 值） | `api` | `api.type`（3 值） | 由 URL 后缀表达（3 值） | 由 provider id + `wireApi` 表达（3 值） |
| Base URL | `options.baseURL` | `baseUrl` | `baseUrl` | `baseURL` | `api.baseUrl` | `url` | `baseUrl` |
| 密钥 | `options.apiKey` | `apiKey` | `apiKey`（环境变量名或字面量） | `apiKeyEnv` + `.credentials.yaml` | `access.apiKey` | `apiKey` | `envKey` + 顶层 `env` |
| 超时 | `options.timeout` | — | — | `timeoutMs` | — | — | `generationConfig.timeout` |
| 重试 | — | — | — | `retryPolicy.mode` / `maxRetries` | — | — | — |
| 上下文 / 输出 | `limit.context` / `limit.output` | `contextWindow` / `maxTokens` | 同 pi | 同 pi | `properties.contextWindow` / `optionSpecs.maxOutputTokens.max` | `maxInputTokens` / `maxOutputTokens` | `generationConfig.contextWindowSize` / `generationConfig.samplingParams.max_tokens` |
| 输入模态 | `modalities.input` | `input` | `input` | `input` | `properties.supports*` | `supportsImages` | `capabilities.vision` |
| 推理档位 | `variants`（none…ultra） | `thinkingLevelMap`（off…ultra） | `thinking.efforts` | `reasoningEfforts` | `optionSpecs.reasoningLevel.values` | `supportsReasoning`（布尔，无档位） | `capabilities.reasoning.efforts` |
| 模型启用 | — | — | — | — | `config.enabled`（ZCode 自管，本工具不接管） | `disabled` | 停用即不写入（见下） |
| 模型存储 | Map（键 = model id） | Array（含 `id`） | 同 pi | 同 pi | 独立规则表 | Array（含 `id`） | Map（pid → Array，含 `id`） |
| Agents 区块 | 支持 | — | — | — | — | — | — |

协议（`npm` / `api`）在 opencode 系 / pi / omp / DSH 四类格式间共用同一份数据，判定顺序为：`npm` 非空按 npm 包推导 → `api` 非空直接用 → 原文件的 `api` → 都没有则按兼容层 `openai-completions`。下拉首项「(空)」表示不指定协议，与 opencode 页 `npm` 的空选项同义且跨页同步。

Qwen Code 的协议落在 **provider id（`modelProviders` 的键）加条目自己的 `wireApi`** 上，不与上面那份数据共用：内置 id `openai` / `anthropic` / `gemini` / `vertex-ai` 直接就是协议，OpenAI 的两种 API 共用 `openai` 这个 id、靠 `wireApi`（`chat-completions` / `responses`）区分；自定义 id 必须在顶层 `providerProtocol` 里声明映射，否则 Qwen Code 会**把整条静默跳过**。`qwen-oauth` 这个 id 是硬编码的、不可覆盖，该 id 下的条目在界面上不显示、保存时整块原样保留。

## 获取模型

每个 provider 卡片与「新增 Provider」弹窗的 Models 标题右侧都有「获取模型」按钮，按 provider 的协议类型请求模型列表并弹层展示：

- 展示为 checkbox 网格，列数按面板可用宽度自适应（窄窗口也不横向溢出），高度固定，超出部分在卡片内滚动；请求中显示进度指示
- 模型 id 过长时截断显示，悬停可看完整名称
- 已配置的模型自动勾选；勾选未配置的模型即新增，取消勾选不会删除已有配置
- 兼容 `data` / `models` / 裸数组三种响应结构（含 `models/` 前缀清理与去重）

## Agents

Agents 区块只在 opencode 系三页出现。agent 可增删改复制、拖拽排序、折叠展开，字段包括 key / mode / description / model / variant / temperature / color / system。

`model` 下拉的候选按三段固定顺序排列：

1. **当前页面自家网关的模型**（排最前）；
2. **已配置 provider 的模型**（保持用户自己的配置顺序）；
3. **该 agent 当前的取值**（若前两段都没有它）。

第三段永远保留：某个已配置的模型被上游下架后，若直接从候选里抹掉，用户会看到「下拉里选中项不见了」，误以为配置坏了。保留它就能看见自己配的是什么，想换再换。

### 各页的自家网关与免费模型

| 页面 | 网关模型前缀 | 免费模型 |
|---|---|---|
| opencode | `opencode` | models.dev 标记免费、且 Zen 网关仍在提供的那些 |
| Kilo Code | `kilo` | Kilo 网关标记为免费的 |
| MiMo Code | `mimo` / `xiaomi` | 无免费层，只列用户自己配的 provider 模型 |

免费模型**不写死在程序里**，而是按页面动态获取（免费层会随上游上下架，写死的 id 迟早变成「选了却跑不起来」的过期项），结果缓存 24 小时；下拉旁显示当前条数，并带一个「刷新」按钮可随时强制重取。请求失败时继续用已有列表，不会让下拉变空。

**MiMo Code 没有免费层**：它的免费模型挂在订阅套餐（Token Plan）下，与「不花钱就能用」不是一回事，所以这一页不显示免费模型提示与刷新按钮。

**三页互不串台**：opencode 页只列 Zen 网关的模型、Kilo Code 页只列 Kilo 网关的，因为各自的网关只认自己的模型 id。

### 切页会按目标页网关调整 agent 的 model

`model` 的前半段必须是**目标页网关认的 provider id**。把 opencode 页配好的 `opencode/…` 带到 Kilo Code 页，Kilo 网关不认这个前缀，agent 直接跑不起来——而界面看不出问题。所以切到 opencode 系页面时，指向**别家网关**的 model 会被换成该页自家网关的首选模型，并在状态栏提示替换了几条。

- 指向**用户自己配的 provider** 的引用在任何一页都有效，不动；
- 空 `model` 不动（那是「还没选」，不是「选错了」）；
- 每页各记一份自己的取值：切走再切回，看到的是你在那一页配的值，不会被重置成默认值；
- 「一键保存」写往各页时同样按目标页调整，每个文件都拿到自己网关认的值。

## 延迟 / 连通性测试

- **厂商连通性**：Providers 标题行右侧「连通性测试」按钮，一键测试当前页面全部厂商，耗时显示在各厂商卡片名字右侧（失败显示错误码，悬停看完整错误）；卡片收起时依然可见
- **模型延迟**：每个模型行的「测试」按钮，**一次只测一个模型**，没有批量入口
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
- **像真实客户端**：一律流式、测到首字后继续读到流结束才断开、`User-Agent` 按协议伪装成白名单客户端（Anthropic → `claude-cli/…`，其余 → `opencode/…`）。中转站普遍只放行白名单客户端
- **token 上限 16**：避免推理模型因上限过小返回空内容或整体报错
- 节流状态只在内存中，重启清零；测试仍会消耗极少量 token，不建议在计费敏感账号上频繁测试

## 查询用户数据

Providers 标题行的「查询用户数据」按钮会对当前页面的 provider **一次性取回所有能取到的账号数据**：余额、已用、今日 / 近 7 天用量、请求次数、分组，以及**签到状态**。请求优先级为：令牌额度 → 令牌调用日志 → 兼容账单接口。

前两个接口互相独立：有些站点的 baseUrl 指向中转域名、只挂了 relay 路由，额度接口会失败但日志接口照样可用——此时今日 / 近 7 天与模型拆分照常显示，只是没有额度三项。两个都拿不到时才回退兼容账单。

请求只读管理接口并直连（不走系统代理）；凭证只放入请求头，不写入日志、状态栏或错误文本。同一 provider 两次查询至少间隔 5 秒。占位额度和不限额令牌只报已用、不给余额；有就显示、没有就不显示。

今日用量按本机时区从当天 0 点计算；跨过本地午夜后「今日」数字会隐藏，累计 / 余额 / 近 7 天保留，不会因此自动发起请求。日志只取首个分页，未遍历后续页时统计可能偏小。

悬停详情按两套口径分节：**账号级**（面板令牌）给余额 / 已用 / 请求次数 / 分组，**令牌级**给累计已用 / 余额 / 额度 / 今日已用 / 今日模型 / 近 7 天；末尾带两个会改变读数的提示——换算比假设（站点没给换算比时按 1 美元 = 500,000 quota 估算）与跨日提醒。

### 站点面板令牌（PAT）

上面的接口都只需 `sk-` key，但公益站的额度常是占位值（拿不到真实余额）。要拿**账号级**真实余额，在标题行点「令牌」为站点填写面板访问令牌：

- 打开的是**可拖动 / 可关闭的悬浮窗**（不占正文布局，可与右侧预览面板同时开着）；下次打开按已保存值重填
- 在哪生成：站点面板「**个人设置 → 安全设置 → 系统访问令牌**」；登录后访问 `GET /api/user/token` 也会生成一串
- 它是**站点 / 账号级**的，不是 provider 级：同一站点的多个 provider 共用一份，面板里按站点列一行并标注共用的 provider
- 站点只挂了中转路由时，面板令牌是拿到余额的**唯一途径**
- 与 `sk-` 是两套凭证：`sk-` 用于推理与令牌额度接口，面板令牌用于账号接口（只读）。把 `sk-` 填进去会得到「令牌无效或已撤销」
- **用户 ID（可选）**：部署**旧版 new-api** 的站点会在账号接口上回 `401 New-Api-User header not provided`——令牌没问题，缺的是防跳站校验头。此时填上用户 ID（面板里 F12 看任意一次账号请求的 `New-Api-User` 值）即可；新版不需要，**留空就不发这个头**。填错会回 `does not match logged in user`；上次查询回过「缺头」的站点会把那行标黄
- 令牌无效 / 站点未开该接口 / 返回不可识别都**不影响**原有的 `sk-` 结果

## 签到状态

部分站点（new-api 的「签到」功能）每天可领一份随机额度。本工具**只读状态、不代签**——签到会改账号额度并在站点记系统日志，所以这里没有任何写入口。状态并进「查询用户数据」的结果一起显示。

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
- 标题行的「**对比**」切换到逐行差异视图（见下）

### 对比视图

标题行的「对比」把待保存文档与**磁盘上的目标文件**逐行比对，直接回答「按一下保存会改掉什么」——跨格式写入会整段接管 provider 容器、同格式写入也会重排条目，光看最终文档看不出改动落在哪。

- 改动块上下各保留 3 行上下文，块头是 `@@ -旧起,行数 +新起,行数 @@`（与 `diff -u` 同形）
- 顶部给 `+新增 / -删除` 计数；两侧没有公共行时额外标「（整份变更）」，通常是新建文件或整体重写
- 目标文件不存在或读不出时按空内容比对，整份文档显示为新增——正是「这个文件还没有，保存会创建它」的含义
- 基线是**磁盘当前内容**，所以保存之后对比自然变空，表示「没有待保存的改动」
- 只读视图：不渲染文本框，因此不存在手改被对比覆盖的问题；切回编辑模式草稿原样保留
- 改动后约 0.5 秒才刷新差异：草稿每敲一个字符都变，逐键重算会在长文件上拖慢输入

## 配置体检

保存按钮那一行右端的「**体检**」按钮把散落在各处的检查汇总成一张清单，一次看完当前页有什么问题：

| 结论 | 检查项 |
|---|---|
| **会阻止保存** | agent 名重复、provider key 重复、同一 provider 内 model id 重复（保存会被拦下） |
| **需要注意** | provider 没有 key、model 没有 id、数字字段非法（timeout / context / output / temperature）、baseUrl 写法可疑、agent 的 model 指向本页不认的网关 |
| **提示** | provider 未填密钥、agents 不会写进本页格式 |

- 按钮上带问题数量，颜色随严重度变化（红 = 有阻断项，黄 = 只有需注意项），不必点开才知道
- 每条给出定位（provider key / agent 名 / 条目）与处理建议，清单按严重度排序
- **只列问题，不提供一键修复**：这些问题的正确修法取决于用户意图（重复的 model id 该留哪条？可疑的 baseUrl 该不该改？），自动改等于替用户做决定
- 判定口径与保存完全一致（复用同一套函数），不会出现「体检说没问题、保存却失败」
- 「agent 的 model 本页网关不认」这一条尤其值得提前看：这种引用在切页与保存时会**被静默替换**成目标页网关的首选模型（见「Agents」一节），先看见比事后发现好
- 未填密钥只是提示：本地推理与公开网关本就不需要密钥

## 保存与 WSL 同步

- 每页有独立保存按钮与写入路径，默认写 Windows 本地路径
- 保存按钮这一行**右端**是三个按钮：**令牌 · 显示密钥 · 预览**（分别管当前页站点的面板令牌、所有页面的密钥显隐、当前页的待保存文档）
- **一键保存**（按钮带目标数量）把同一份界面状态一次写进每个已安装后端的目标路径，免去逐页切换逐个点保存；未安装的后端跳过，不去凭空创建配置文件
- 当前文件属于本页格式且已加载时写当前文件；手动改了路径但未加载时写入该路径并保留目标文件其余配置
- 手动指定过的路径按页面记住（写进设置文件）：启动时优先打开「覆盖过且文件确实存在」的页面；把路径输入框清空后回车即清除该页覆盖，回到自动探测的默认路径
- **启动时按文件内容判定方言**，而不是只按「哪一页填了路径」：把 opencode 的配置文件填到 pi 页时，用 pi 方言去读会解析出 0 条 provider（表现为「写了路径却不自动加载」）。现在启动探测与手动加载走同一套内容判定，页面会跟随实际格式切换
- 跨格式写入由界面接管的容器：`provider`（opencode 系）/ `providers`（pi / omp / DSH）——容器内的条目与顺序完全来自界面，目标文件里多出来的旧条目不残留；目标文件其余顶层配置（如 `mcp`、`instructions`）原样保留
- opencode 的 `agent` 容器只在界面确实持有 agents 数据时才接管；来源为其他格式时界面无从表达 agents，**目标文件已有的 agents 原样保留**，不会被清空
- 跨格式写入覆盖已存在的文件前，先把原内容备份为 `<文件>.bak`（内容相同或文件为空时跳过）；备份失败则取消保存，不会静默替换旧配置
- 勾选「WSL同步」后同时写入 WSL 侧对应路径；未在 WSL 中安装对应 agent 时禁用勾选

## 界面设置与状态记忆

工具的界面设置写在**家目录**的 `.modelharbor/settings.json`（Windows：`%USERPROFILE%\.modelharbor\settings.json`）。
里面只有界面选择：主题、形状、保存格式、密钥显隐、WSL 同步、卡片折叠集合、代理支持开关、各页路径覆盖、页签顺序。
**不存密钥、不存模型、不存配置内容**（配置永远以你自己的 agent 配置文件为真源）。

- 写入策略：内容没有变化就不写盘；有变化时先写同目录临时文件并同步，再替换正式文件；替换失败会保留原设置。读不出 / 解析失败 / 文件不存在都按默认值处理
- 取不到家目录时回退到 `%APPDATA%\.modelharbor\`，再回退到程序同级目录；任何情况下都不会写进 agent 配置目录
- 删除该文件即恢复默认界面设置

### 页签顺序

已安装的页面排在前面，未安装的按名字排在后面。已安装的那一段可以**拖动换位**，顺序会被记住。拖动中光标保持握拳直到松手；状态用颜色区分：选中 = 悬浮色 + 加粗描边，正在拖动的页签 = 橙色，换位目标 = 绿色。

### 卡片折叠状态

- 记录的是**折叠**的卡片，不在记录里的就是展开：新加载的卡片默认展开
- 折叠键按「配置文件身份 / 类别 / 名字」分区；同一份配置在各页面里共享状态，不同配置文件互不覆盖
- 加载成功后只清理当前配置身份里已经不存在的记录（删掉 / 改名后不残留）；其他配置身份和加载失败场景都保留
- v2 的旧键会在首次成功加载配置后迁移到当前配置身份

### 主题与形状

顶栏「外观」面板里两组选择，**两者正交**：主题管颜色，形状管控件长相。

**主题**（8 种，按底色分两行）：

- 深色系：深色 / 海洋 / 极地 / 苔藓
- 浅色系：亮色 / 玫瑰 / 薄荷 / 薰衣草

各自一套底色与强调色，选中后立即生效并在下次启动时恢复。状态色（绿 = 正常 / 黄 = 注意 / 红 = 异常 / 蓝 = 信息）**跨主题保持一致**，只在蓝与当前主题强调色过于接近时改用青蓝，保证在每种底色上都看得清。每个主题的正文 / 强调 / 描边都按 WCAG 下限校过（正文与强调 4.5:1、描边 3:1）。

**形状**（8 种，4+4 两行）：

| 形状 | 观感 |
|---|---|
| 标签 | 胶囊全圆角 + 0.5px 细边 |
| 圆润 | 圆角 10 + 1px 描边（默认档） |
| 精致 | 圆角 6 + 0.5px 细边，轻量感 |
| 极简 | 全直角 + 无边框，纯色块 |
| 云朵 | 大圆角 16 + 软投影，卡片浮在底上 |
| 浮雕 | 凸起受光线 + 接触影 |
| 石板 | 平放的板——深色描边 + 接触影 |
| 色带 | 卡片顶部 3px 主题强调色条 |

无边框的形状（极简 / 云朵）悬停时会在卡片内侧浮现 1px 高亮环。

旧设置里已删除的形状键（`heavy` / `sharp` / `compact` / `neon` / `frosted` / `prism`）会自动回落到默认档（圆润），不需要手工清理 `settings.json`。

### 旧版本位置兼容

早期版本把设置放在 `%APPDATA%\.modelharbor\prefs.json`，后来移到家目录且文件名改为 `settings.json`。
读取时按「家目录 settings.json → 家目录 prefs.json → `%APPDATA%` 下两者」依次回退，
写盘只写新位置的新名字，并在首次保存成功后清掉家目录同目录的旧文件（老设置不会丢）。

## 缺省值与字段映射

- 配置未写 `timeout` / `timeoutMs` 时显示默认 `180000` ms，未修改时不写回
- DSH 的 `retryPolicy.mode` 缺省显示 `normal`
- pi 的 `compat.requiresReasoningContentOnAssistantMessages` 与 omp 的 `compat.requiresReasoningContentForAllAssistantTurns` 相互映射；加载 opencode / DSH 或新建时默认不勾选

### 上下文 / 输出的预设下拉

上下文字段右侧的预设下拉给出常用的上下文值（128k / 200k / 262k / 300k / 400k / 500k / 1024k），输出字段右侧同理（32k / 64k / 131k / 262k）。选中即填入，**填完仍可继续手动编辑**——它只是省去手敲大数字，不锁定字段。

预设按 **1000 进制**换算成配置里的整数（`128k` → `128000`），与仓库既有的写法一致（新模型的占位值就是 `262000` / `131000`）。不用 1024 是因为这些字段是**发给上游的声明**：厂商文档与网关界面普遍按 1000 报数，少声明一点是安全的，多声明会被上游直接拒绝，所以 `262k` 落 `262000` 而不是 `262144`、`1024k` 落 `1024000` 而不是 `1048576`。输出里的 `131k` / `262k` 同理，对应厂商常声明的 `131072` / `262144`。

下拉显示「选择...」表示当前值不在预设里（手填的值不会被改成某个预设）。

## 配置文件格式参考

字段细节由界面表单呈现，这里只说明各格式的**结构形状**（加新后端时按同一张表扩展）。

**JSON 系**——opencode 系三页的 `provider` 与 `models` 都是 **Map**（键 = 名字 / model id），schema 相同，此处以 opencode 为例；pi 的 `providers` 是 Map、`models` 是 **Array**（每项含 `id`）；WorkBuddy 的根就是一个 **Array**，provider 信息内联在每条模型里：

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

// ~/.workbuddy/models.json —— 根是数组，一条目一模型
[ { "id": "…", "name": "…", "vendor": "…", "url": "…", "apiKey": "…",
    "supportsToolCall": true, "supportsImages": true, "supportsReasoning": false,
    "useCustomProtocol": false, "maxInputTokens": 0, "maxOutputTokens": 0 } ]

// ~/.qwen/settings.json —— modelProviders 的键是 provider id，值是数组；
// 一条元素 = 一条完整路由（自带 baseUrl / envKey），密钥在顶层 env
{
  "modelProviders": { "openai": [
      { "id": "…", "name": "…", "baseUrl": "…", "envKey": "OPENAI_API_KEY",
        "wireApi": "chat-completions",
        "capabilities": { "vision": true, "reasoning": { "efforts": ["low", "high"] } },
        "generationConfig": { "timeout": 60000, "contextWindowSize": 0,
                              "samplingParams": { "max_tokens": 0 } } } ] },
  "providerProtocol": { "<自定义 id>": "openai" },
  "env": { "OPENAI_API_KEY": "…" }
}
```

**ZCode**（`~/.zcode/v2/provider_config.json`）的 provider 与模型级属性分两处存放：`providerConfigRules` 只列模型 id，每个模型的元数据在 `modelConfigRules.providerModelRules` 里。界面编辑时两边同步，不必手工对齐。

```jsonc
{ "schemaVersion": 1, "config": {
    "providerOrder": ["<providerId>"],
    "providerConfigRules": { "providerRules": [ {
        "providerId": "…", "providerName": "…",
        "config": { "group": "standard-personal",
          "access": { "type": "api-key", "apiKey": "…" },
          "api": { "type": "openai-chat-completions", "baseUrl": "…" },
          "personalModelIds": ["…"], "modelOrder": ["…"] } } ]},
    "modelConfigRules": { "providerModelRules": [ {
        "modelId": "…", "providerId": "…",
        "config": { "enabled": true,
          "properties": { "contextWindow": 0 },
          "optionSpecs": { "maxOutputTokens": { "max": 0 } } } } ] } } }
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

- **「启用」开关出现在 WorkBuddy 与 Qwen Code 两页**（其余七家的模型 schema 里没有模型级启用字段）。两家的共同点是：生效清单只含启用的条目，**关掉不删配置**。
  - **WorkBuddy** 的选择器按模型 id **全局去重**，同一个模型名无论挂在哪个厂商下都只会列出一行、只有第一条生效，所以开关是**全局互斥**的——打开一个，同名的其他条目自动关闭。只有开启的会写进 `models.json`；关闭的条目仍保存在同目录的 `models.full.json`，开回来即恢复。
  - **Qwen Code** 的 schema 里没有 `disabled` 字段，**停用就是整条不写进 `settings.json`**；全部条目与勾选状态记在同目录的 `modelProviders.full.json`，开回来即恢复。
- **WorkBuddy 的模型 `id` 就是发给上游的模型名**，不要为了区分同名模型去改它——改了会直接请求失败。要用哪一家，就在那一家的卡片上打开「启用」。
- **两家各有一个伴生文件**（WorkBuddy 的 `models.full.json`、Qwen Code 的 `modelProviders.full.json`，都与主配置同目录），保存全部条目与勾选状态，含 API key。**不要手工删除它们**：删了之后未勾选的条目会从界面上消失。两家都只按精确文件名读自己的主配置，同目录其他文件不看。
- **Qwen Code 的密钥存在主配置顶层的 `env` 里**（条目上只写变量名 `envKey`），本工具**只增改、绝不删**：那个命名空间是跨 provider 共享的，Qwen Code 自己的 `/auth` 也往里写（例如 Coding Plan 的 `BAILIAN_CODING_PLAN_API_KEY`），按「有没有条目引用」去修剪会把刚配好的凭据静默删掉。
- **Qwen Code 的 `providerProtocol` 改动需要重启 Qwen Code 才生效**（`modelProviders` 是热加载的，协议映射只在启动时读一次）。
- **ZCode 的模型开关由 ZCode 自己维护**：你在 ZCode 界面里停用的模型，ModelHarbor 保存时不会把它重新打开。
- **数字字段填错不会写坏配置**：非法输入会被标红提示，保存时该字段被忽略（状态栏会报忽略了几项），不会把上下文写成 `0`。
- `baseURL` 末尾 `/v1` 的归一化按目标 agent 的客户端行为决定，**读入与写出都做**：pi / omp / DSH 的 `anthropic-messages` **去掉**末尾 `/v1`（这三家客户端自己拼 `/v1/messages`）；opencode 的相反，baseURL **必须带** `/v1`。其他协议一律不动。
- **跨页保存会按目标页调整 agent 的 model**：指向别家网关的引用会被换成目标页自家网关的模型（见「Agents」一节）。各页自己配的值会被记住，切回来会还原。
- provider / model 只保存各自支持的字段，方言字段不会互相泄漏
- omp 的 `apiKey` 为「环境变量名或字面量」语义；推理档位保存为官方 `thinking` 块
- 保存 YAML 时文件注释不会保留，输出为标准块风格
- DSH 的实际密钥保存在同级 `.credentials.yaml`，加载时自动读取、保存时写回；凭据文件中的其他字段原样保留

## 平台与安全

- 当前**仅支持 Windows**
- 配置文件中的 `apiKey` 为**明文**（DSH 的 `.credentials.yaml`、WorkBuddy 的 `models.full.json`、Qwen Code 的 `env` 与 `modelProviders.full.json` 同样），请勿提交到公开仓库
- 界面设置文件只保存界面选择、不含密钥，可以安全删除（会恢复默认界面设置）
- 站点面板令牌存在 `%USERPROFILE%\.modelharbor\tokens.json`，**含凭证且为明文**（与 agent 配置文件同级风险），请勿提交或同步到共享目录。里面只有你主动填过的站点，在「令牌」面板点「删除」或直接删除该文件即可清空
- 令牌与用户 ID **不会**写进 `settings.json`，也不会写进任何 agent 配置文件；接口请求只把它们放进请求头（不进 URL、不进日志与状态栏文本）
- ZCode 只接管 `provider_config.json`：内置 provider 的 `config.json`、加密凭据 `credentials.json`、只读模型库 `zcode-builtin.json` 都不属于可编辑面，不会被写入
