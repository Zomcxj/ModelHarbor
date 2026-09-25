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

[Kilo Code](https://kilo.ai) 与 [MiMo Code](https://mimo.xiaomi.com/coder)（小米）都是 opencode 的 fork，配置 schema 的**字段与结构一致**——顶层 `provider` / `agent` 两个容器、`options.baseURL` / `options.apiKey` / `options.timeout`、`models.<id>.limit.context|output`、`tool_call`、`reasoning`、`modalities.input` 全部一致。因此这三页共用**同一套**解析、序列化、字段可见性与 Agents 支持实现，差别只有三项：

| | 配置目录 | 主配置文件名 | `$schema` |
|---|---|---|---|
| opencode | `.config/opencode/` | `opencode.json`（也接受 `opencode.jsonc`） | `https://opencode.ai/config.json` |
| kilocode | `.config/kilo/` | `kilo.json`（也接受 `kilo.jsonc`） | `https://app.kilo.ai/config.json` |
| mimocode | `.config/mimocode/` | `mimocode.json`（也接受 `mimocode.jsonc`） | `https://mimo.xiaomi.com/mimocode/config.json` |

因为三者内容形状完全一致，**没有任何内容特征能区分它们**，判别只能靠**路径**：命中目录名或文件名即认领对应页面，没有路径线索时才按内容（顶层 `provider` 对象）回落 opencode。后果是：把 `kilo.json` 的内容复制到 `opencode.json`，会被认作 opencode 页面——这不影响正确性，因为三者写出的字段结构完全相同（只有 `$schema` 的 URL 按各页取值），只是图标与保存路径跟着文件名走。

配置目录名以各家官方文档为准：Kilo Code 读 `~/.config/kilo/kilo.json`（另外兼容读取同目录下旧版 `opencode.json`，但不读 `.opencode/`）；MiMo Code 读 `~/.config/mimocode/mimocode.json`（同目录也接受 `config.json`），不读 `opencode.json`。

**首次运行生成的可能是 `.jsonc`，页面必须照样认。** 两个 CLI 第一次启动时写的是 `.jsonc` 变体（实测 `kilo.jsonc` / `mimocode.jsonc`），而默认路径指向 `.json`。若只认默认名，会同时错两处：页面判成「未安装」（页签变灰、一键保存跳过它），保存还会**另建**一个 `.json`，把用户真正的配置晾在一边。所以路径知识统一走「候选列表」：`Backend::path_candidates()` 给出 `[主名, .jsonc 变体]`，`resolve_local_path()` 取其中第一个真实存在的文件（都不存在才用主名，供新建）。`local_available()`、启动探测 `ConfigPaths::detect()`、WSL 批量探测、保存目标 `refresh_targets()` 全部走这条解析。两个细节：候选只替换**文件名**那一段（父目录里恰好出现同名字串时不能误替换），且解析结果再喂回去必须幂等（不能滚出 `.jsonc.jsonc`）；WSL 侧优先取「确实是文件」的候选，否则在「父目录存在即算安装」的宽松判定下会挑中并不存在的 `.json`。

**`$schema` 会写进配置。** CLI 自己生成的配置就带这个字段（`kilo.json` 里只有一行 `$schema`），它让编辑器与 CLI 拿到字段补全。此前只在目标文件已有该字段时才会保留，新建 / 合并到不存在的目标写出的是一份裸的 `{"agent":{},"provider":{}}`，预览里看不到 `$schema`。现在序列化时统一补齐各家官方地址并排在**首位**；**已有值一律保留**——用户可能手动改成镜像地址，覆盖等于替用户改配置。

**三家的 `required` 并不相同，写盘时按目标方言补齐。** 三个 CLI 都用 zod 校验配置，schema 里的 `required` 是**真会拒绝启动**的（实测 `mimo models` / `kilo models` / `opencode models` 对半截 `limit` 一律报 `Missing key … limit.output`）。而必需字段各家有别：

| 字段 | opencode | kilo | mimocode |
|---|---|---|---|
| `limit.context` + `limit.output` | 必需 | 必需 | 必需 |
| `modalities.input` + `modalities.output` | 可选 | 可选 | **必需** |

于是同一份配置在三页之间**并不等价**：opencode 允许模型只写 `modalities.output`，原样写进 mimocode 就是一份**非法文件**——用户实测过这个（opencode 里那几个只写 `output` 的模型，切到 mimo 页保存后 `mimo` 拒绝加载，报 `expected array, received undefined … modalities.input`）。所以序列化后统一过一遍补齐，且**只按目标方言**补：给 opencode / kilo 补 `input` 会是凭空添加用户没写过的字段。

补法按「缺的那一侧有没有安全的默认值」分两种，不搞一刀切：

- **`modalities`**：缺的一侧补 `["text"]`。整块省略时 CLI 本来就是按「纯文本」理解的，补 `text` 是对源方言语义的忠实表达，同时保住用户明确写出的那一侧。两侧都没写（`{}`）则整块删掉——mimo 对空对象会同时报两个缺失，而**没有** `modalities` 键是合法的。
- **`limit`**：半截 limit **无法**用合法值表达（schema 要数字，而「不限」没有对应值），凭空编一个上下文窗口比交给 CLI 自己的模型库更糟，所以整块删掉；`limit` 在模型级本就可选，省略后 CLI 用它自己的数据（实测三家都接受省略）。

补齐覆盖**整个目标文件的模型**，不只是界面接管的那些：合并写入时目标文件里原有的条目会保留下来，它们同样要过校验，否则照样是「CLI 起不来」。

**目录名优先于文件名**：Kilo 那份遗留的 `~/.config/kilo/opencode.json` 名字像 opencode，但目录已经把它判给了 kilocode，所以归 kilocode 页面（反过来 `~/.config/opencode/kilo.json` 归 opencode）。否则按注册顺序先到的 opencode 会抢走它，页面与保存路径都会指错目录。

**MiMo Code 的页签图标不是官方 favicon。** MiMo Code 是 opencode 的 fork，它仓库里的 favicon、桌面应用图标、console logo 全都沿用 opencode 的同一份图形（`favicon.svg` 与 opencode 官方逐字节相同），照抄官方资产会得到一个和 opencode 页签**看起来一模一样**的图标。所以这里改用小米官方 logo（橙底白色 `mi`）。测试 `opencode_family_members_have_distinct_paths_and_icons` 因此按**像素差异占比**（阈值 25%）判定，而不是只比字节——同图形换抗锯齿只有约 13% 差异，换图形可达 97%。

## 获取模型

每个 provider 卡片与「新增 Provider」弹窗的 Models 标题右侧都有「获取模型」按钮，按 provider 的 api 类型请求模型列表并弹层展示：

- 展示为 checkbox 网格，列数按面板可用宽度自适应（窄窗口也不横向溢出），高度固定，超出部分在卡片内滚动；请求中显示进度指示
- 模型 id 过长时截断显示，悬停可看完整名称
- 已配置的模型自动勾选；勾选未配置的模型即新增，取消勾选不会删除已有配置
- 兼容 `data` / `models` / 裸数组三种响应结构（含 `models/` 前缀清理与去重）

## Agents 的 model 下拉与免费模型

opencode 系（opencode / kilocode / mimocode）页面的 Agents 区块里，`model` 下拉的候选按**三段固定顺序**排列：

1. **当前页面自家网关的模型**（`<网关 provider id>/<id>`，排最前）；
2. **已配置 provider 的模型**（`provider/model`，保持用户自己的配置顺序）；
3. **该 agent 当前的取值**（若前两段都没有它）。

顺序本身是需求：**自家网关必须排最前**。早先的实现把三段混在一起整体排序，`kilo-auto/free` 这类网关里的关键项会被 `kilo/~anthropic/…` 按字典序挤到后面，「前几家」也就不是自家可用的了。现在只去重、不排序，各段内部保持给定顺序（网关段由 `gateway_models` 决定，首选在最前）。

第三段**永远保留**：某个已配置的模型被上游下架后，若直接从候选里抹掉，用户会看到「下拉里选中项不见了」，误以为配置坏了。保留它就能看见自己配的是什么，想换再换。

免费模型**不写死在代码里**，而是动态获取——免费层会随上游上下架，写死的 id（早期版本是 `opencode/mimo-v2.5-free` 与 `opencode/big-pickle` 两个）迟早变成「选了却跑不起来」的过期项。实现见 `src/opencode_models.rs`。

### 各后端的免费模型来源

| 后端 | 引用前缀 | 免费模型来源 | 判定 |
|---|---|---|---|
| opencode | `opencode` | models.dev 的 `opencode` 厂商 + Zen 网关可用性 | 价格为零 **且** 网关在供 |
| kilocode | `kilo` | Kilo 网关 `api.kilo.ai/api/gateway/models` | 响应里的 `isFree` 字段 |
| mimocode | `mimo` / `xiaomi` | **无免费层**，只列用户自己配的 provider 模型 | — |

引用前缀就是各网关自己的 provider id，与 agent 配置里 `provider/model` 的写法同源（MiMo 官方文档的 `xiaomi/mimo-v2.5-pro` 即此规则）。**三页互不串台**：opencode 页只列 Zen 网关的模型、kilocode 页只列 Kilo 网关的，因为各自的网关只认自己的 id。

**MiMo Code 没有免费层。** models.dev 上它对应的 `xiaomi` 厂商 9 个模型全部收费；免费的那些挂在 `xiaomi-token-plan-{cn,sgp,ams}` 下，那是**订阅套餐**（Token Plan）而不是免费层，与「不花钱就能用」不是一回事。所以 mimocode 页不显示免费模型提示与刷新按钮。

**「自家网关」比「免费层」宽。** `source_for` 只回答「免费层从哪拉」，mimocode 没有免费层却同样有自己的网关。所以下拉排序与下面的切页替换用的是 `gateway_provider_ids`（opencode → `opencode`；kilocode → `kilo`；mimocode → `mimo`、`xiaomi`）与 `gateway_models`。后者优先用动态拉到的免费列表，**列表为空时退回内置兜底**——否则「切页即替换」会因为列表还没拉回来而静默失效。兜底值取各页最稳的那个 id（网关的自动路由优先）：

| 后端 | 兜底模型（动态列表为空时） |
|---|---|
| opencode | `opencode/big-pickle` |
| kilocode | `kilo/kilo-auto/free` |
| mimocode | `mimo/mimo-auto`，其后是 `xiaomi/mimo-v2.5*` / `mimo-v2.6*` 全量（与 `mimo models` 输出一致） |

### 切页即替换：指向别家网关的 model

`model` 的前半段必须是**目标页网关认的 provider id**。把 opencode 页配好的 `opencode/ling-3.0-flash-fin-free` 带到 kilo 页，kilo 网关不认这个前缀，agent 直接跑不起来——而界面看不出问题（下拉里就显示着那串字）。

所以在两个位置各做一次归一，规则同一套（`opencode_models::model_is_valid_on`）：

- **切页时**：进到 opencode 系页面就检查每个 agent，`model` 非空且前缀既不是本页网关、也不是用户自己配的 provider key 时，换成该页自家网关的首选模型。界面立即可见，状态栏提示替换了几条。
- **写往别的页面时**：保存（含「一键保存」）与右侧预览都按**目标页**归一。三页共用同一份 agents 数据，不按目标页分别归一的话，最后访问过的那一页的模型会被写进所有文件。当前页写自己加载来的数据则原样落盘——那份在切页时已经归一过并显示给用户看过。

判据里「用户自己配的 provider key」在任何一页都算有效：保存时 provider 容器一并写进目标文件，引用不会落空。空 `model` 不动（那是「还没选」，不是「选错了」）；拿不到任何自家模型时也一律不动，宁可少替换也不把配置清成空串。

### 每页各记一份 agent model（否则切走再切回会丢配置）

单向替换有个隐蔽后果：用户从 opencode 页切到 kilo 页（`opencode/…` 被换成 `kilo/kilo-auto/free`），再切回 opencode 页——**原来那个 `opencode/…` 已经没了**。用户什么也没改，配置却变了。

所以进程内另存一份 `agent_models_by_page`（页面 → agent 名 → model）：**离开一页时**记下该页当前的 model 视图，**回到该页时**先还原、再对仍然无效的引用做替换。于是「各页各配各的」能同时成立：

- 用户在 kilo 页挑了 `kilo/kilo-auto/balanced`，切走再切回，看到的还是 `kilo/kilo-auto/balanced`（不是被重置成默认值）；
- 一键保存写到 kilo 的那份，用的是用户在 kilo 页挑的值，而不是当前页那份被归一过的引用。

三条边界：当前页的权威值永远是 `agents` 本身（用户可能正在编辑，记忆对它已过期）；记忆只在离开页面时写入，所以**首次**进入某页没有记忆可还原，按无效引用替换；重新加载配置或从预览内容重建时**清空**记忆——键（agent 名）可能已经不存在，留着会把陈旧的值盖到新文件上。

### opencode 为什么要两个源求交

| 源 | 提供 | 用途 |
|---|---|---|
| [models.dev](https://models.dev) `api.json` | 价格（`cost`）与上下架标记（`status`） | 判断哪些**免费** |
| `opencode.ai/zen/v1/models` | 当前真实提供的模型 id | 判断哪些**还在供** |

单靠任何一个都不够：Zen 的接口只有 id、没有价格，分不出免费与收费；models.dev 的价格准确，但 `deprecated` 标记**偏保守**——实测 `mimo-v2.5-free` 被标记为 deprecated，Zen 网关却仍在正常提供（返回 403 FreeTierError，意思是「模型存在，只是限定在 opencode 内使用」）。所以判定取**两者交集**：models.dev 说免费 **且** 网关确实提供。这样既不会推荐已下架的 id（`glm-5-free` / `kimi-k2.5-free` / `grok-code` 在网关上返回 401「Model is not supported」），也不会漏掉仍可用但被保守标记的模型。网关请求失败时回退为「只信 models.dev，并排除 deprecated 标记的条目」——拿不到实测依据时宁可少列几个。

免费判定要求 `cost.input` 与 `cost.output` **都是显式的 0**；字段缺失说明数据源还没收录价格，按收费处理，避免把收费模型当免费的推荐出去。

Kilo 侧用网关响应自带的 `isFree` 字段，而不是「id 以 `:free` 结尾」或价格推断：`kilo-auto/free` 与 `openrouter/free` 两个免费项并不带 `:free` 后缀，按后缀筛会漏掉它们。

### 缓存

`api.json` 未压缩约 4.8 MB（gzip 后约 470 KB），每次启动都下载太浪费，因此结果**按后端分别**落盘到 `.modelharbor/free-models-<后端>.json`，超过 24 小时才在后台重新拉取。启动先用缓存渲染（不阻塞界面），缓存缺失或过期才在首帧后台刷新。下拉旁的提示会显示当前条数，失败时给出红字与原因，并带一个「刷新」按钮可随时强制重取。拉取失败**不清空**已有列表：宁可继续用旧缓存，也不要因为一次网络抖动让下拉变空。

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
- 写往 opencode 系**别的**页面时，agent 的 `model` 按目标页网关归一（指向别家网关的前缀换成该页自家网关模型，见「切页即替换」）；当前页写自己加载来的数据则原样落盘
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

- **「启用」开关只属于 WorkBuddy 一页**，由 `ConfigFormat::has_model_enable()` 判定。不能改用 `page_has_model_field("disabled")`：那个函数在「已加载的文件格式 ≠ 当前页」时一律返回 true（为的是让切过去的新页面能填所有字段），会把开关漏到每一页——加载 `opencode.json` 时除了 opencode 自己那页全都长出了开关，就是这个原因。
- WorkBuddy 的模型行有一个**启用开关**（滑动开关，写 `disabled` 字段）：WorkBuddy 的选择器按裸 model id **全局去重**，同一 id 只有第一条生效，所以界面上的开关是**全局互斥**的——打开一个，同名的其他条目自动关闭。只有开启的会写进 `models.json`；关闭**不删配置**，条目仍保存在同目录的 `models.full.json`，开回来即恢复。开关用滑动控件而不是勾选框：在密集的模型卡片里勾选框容易被当成装饰，轨道的填充色与滑块位置让状态一眼可辨。点击时滑块**滑动到位**（`motion::TOGGLE_TIME` = 0.14s，轨道填充色与滑块位置同步插值），比悬停过渡略长——滑块要看得见在移动，太快就退化成瞬切。动画状态挂在一个**调用方给出的稳定 id** 上（`("model_enable", model_key)`），不能用 egui 的自动 id：同一行里延迟标签是条件渲染的，自动 id 会随它出现而漂移，动画就串到别的行去了。egui 的 `animate_bool` 对未登记的 id 首帧直接返回终值，所以页面刚打开时开关不会从左边滑进来——只有点击造成的状态变化才走动画
- ZCode 的模型级 `config.enabled`（与 `properties` 平级）是 **ZCode 自己的模型开关**，ModelHarbor 不接管：加载时忽略它、保存时**原值保留**，只在原文件没写该键时补 `true`。曾经无条件写 `true`，会把用户在 ZCode 里关掉的模型重新打开。
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
