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

顶栏图标切换四个页面（opencode / pi / omp / DSH）。加载任意一份配置后，各页面共享同一份数据，修改 provider 参数在所有页面同步生效（provider / model 顺序亦跨页同步）；Agents 区块仅属于 opencode 页面。

各页表单按自身方言显示字段与枚举，无对应字段不显示占位：

- **opencode 页**：`options.baseURL` / `options.timeout` / `npm` 下拉 / `limit.context` / `modalities` / `variants`（none…ultra）
- **pi 页**：`baseUrl` / `apiKey` / `api` 下拉（pi KnownApi 10 值）/ `compat` / `contextWindow` / `maxTokens` / `input` / `thinkingLevelMap`（off/minimal…ultra）
- **oh-my-pi 页**：`baseUrl` / `apiKey` / `api` 下拉（omp 官方 9 值）/ `compat` / `contextWindow` / `maxTokens` / `input` / `thinking.efforts`（minimal…ultra）
- **DeepSeek Harness 页**：`baseURL` / `apiKeyEnv` + 实际密钥 / `api` 下拉 / `timeoutMs` / `retryPolicy.mode` / `retryPolicy.maxRetries` / `models`（`id` / `name` / `contextWindow` / `maxTokens` / `input` / `reasoningEfforts`（minimal…ultra））

协议（`npm` / `api`）四页共用同一份数据，口径统一走 `ProviderRow::effective_api()`：
`npm` 非空按 npm 包推导 → `api` 非空直接用 → 原文件的 `api` → 都没有则按兼容层 `openai-completions`。
下拉首项「(空)」表示不指定协议（`npm` 与 `api` 都清空，写盘时按兼容层处理），与 opencode 页 `npm` 的空选项同义且跨页同步。

## 获取模型

每个 provider 卡片与「新增 Provider」弹窗的 Models 标题右侧都有「获取模型」按钮，按 provider 的 api 类型请求模型列表并弹层展示：

- 展示为 checkbox 网格，列数按面板可用宽度自适应（窄窗口不再横向溢出），高度固定，超出部分在卡片内滚动；请求中显示进度指示
- 模型 id 过长时截断显示，悬停可看完整名称
- 已配置的模型自动勾选；勾选未配置的模型即新增，取消勾选不会删除已有配置
- 兼容 `data` / `models` / 裸数组三种响应结构（含 `models/` 前缀清理与去重）

## 延迟 / 连通性测试

测试请求的端点、鉴权与请求体都按所选协议构造（不再一律打 `chat/completions`）：

| 协议 | 模型延迟端点 | 鉴权 |
| --- | --- | --- |
| `openai-completions` / `mistral-conversations` | `POST {base}/chat/completions` | `Authorization: Bearer` |
| `openai-responses` / `openai-codex-responses` | `POST {base}/responses` | `Authorization: Bearer` |
| `azure-openai-responses` | `POST {base}/responses` | `api-key` |
| `anthropic-messages` | `POST {base}/v1/messages` | `x-api-key` + `anthropic-version` |
| `google-generative-ai` | `POST {base}/models/{model}:streamGenerateContent?alt=sse` | `?key=` 查询参数 |
| `google-vertex` | `POST {base}/publishers/google/models/{model}:streamGenerateContent?alt=sse` | `Authorization: Bearer` |
| `pi-messages` | `POST {base}/messages` | `Authorization: Bearer` |
| `google-gemini-cli` / `bedrock-converse-stream` | 不支持 | 需专有签名 / 私有网关 |

- `anthropic-messages` 的 `{base}` 两种写法都会被归一：已带 `/v1` 时补 `/messages`，未带时补 `/v1/messages`
  （pi / oh-my-pi / DSH 的 base 不含 `/v1`，opencode 的 baseURL 必带 `/v1`，见「注意事项」）

- **厂商连通性**：Providers 标题行右侧「连通性测试」按钮，一键测试当前页面全部厂商，耗时显示在各厂商卡片名字右侧（失败显示错误码，悬停看完整错误）；卡片收起时依然可见
- **模型延迟**：每个模型行的「测试」按钮（在拖动按钮右侧，结果就在按钮右侧），**一次只测一个模型**，没有批量入口
- 测的是**首字延迟**（全程流式）：请求发出 → 第一个带内容的 SSE 块到达；其余协议在同端点靠请求体 `stream: true` 区分，Google 系换动词/加 `?alt=sse`
- 超时判定 10 秒（单个读操作 / 首字的等待上限）；着色：<2 秒绿色、2~5 秒黄色、≥5 秒红色；测试中显示乱码动画
- 不支持自动测试的协议直接报错提示，不发无意义的请求

### 模型延迟的防封号保护

中转站（API 代理）普遍带「多 IP 检测 / 测活封号」风控，所以模型延迟按「单发、低频、像人」设计：

- **网络守卫**：检测到 Windows 系统代理（含 PAC）或处于 `Up` 状态的 VPN / TUN 网卡时，直接禁用模型延迟测试，并在 Providers 标题行给出原因（每 5 秒复查，关掉代理后自动恢复）；厂商「连通性测试」不受影响（只拉模型列表，不做推理）
- **节流（按厂商各自计数，跨厂商不牽连）**：同一厂商的任意两次探测（同模型、不同模型都算）至少间隔 5 秒，且同一厂商同时只允许一个探测在飞（探测最长 10 秒）；不同厂商之间没有间隔，可以并行测。按钮上显示剩余冷却秒数，悬停有说明
- **问句轮换**：请求体是题库里的 24 条跨领域常识名词题（地理 / 天文 / 生物 / 化学 / 物理 / 文学 / 艺术 / 音乐 / 历史，均为「一句话能答」的定论型题目），按 provider key 偏移 + 轮换游标选取，不再固定发 `ping`；**不校验答案**，判定只看「有没有出字」与首字耗时
- **全协议流式**：主流客户端默认全部流式，同步请求在中转站日志里会显示成「类型：同步」，反而是少数派特征；因此探测一律带 `stream: true`（Google 系走 `:streamGenerateContent`），并带上 SDK 惯用的 `Accept: text/event-stream`
- **把流读完**：测到首字后仍继续读到 `[DONE]` / `message_stop` / `response.completed` / EOF（上限 64 KB）才关连接——真实客户端不会拿到流就断，匆匆断开反而像探测流量
- **客户端身份**：`User-Agent` 按协议伪装成白名单客户端（Anthropic / pi-messages → `claude-cli/…`，OpenAI 兼容 / Responses / Google → `opencode/…`）——中转站普遍只放行白名单客户端，实测同一站点同一 key：`ureq/2.x`、不传 UA、`pi/…` 一律 `401 unauthorized client detected`（有的站点直接卡到超时），而这两种身份 200；版本号不被校验
- **token 上限 16**：避免推理模型因上限过小返回空内容或整体报错
- 节流状态只在内存中，重启清零（重启后只能逐个手点，不会批量冲击）；测试仍会消耗极少量 token，不建议在计费敏感账号上频繁测试

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
- 当前文件属于本页格式且已加载时写当前文件；手动改了路径但未加载时写入该路径并保留目标文件其余配置
- 跨格式写入由界面接管的容器：`provider`（opencode）/ `providers`（pi / omp / DSH）——容器内的条目与顺序完全来自界面，目标文件里多出来的旧条目不残留；目标文件其余顶层配置（如 `mcp`、`instructions`）原样保留
- opencode 的 `agent` 容器只在界面确实持有 agents 数据时才接管（来源为 opencode，或在 opencode 页手动新增）；来源为 pi / omp / DSH 时界面无从表达 agents，**目标文件已有的 agents 原样保留**，不会被清空
- 跨格式写入覆盖已存在的文件前，先把原内容备份为 `<文件>.bak`（内容相同或文件为空时跳过）；备份失败则取消保存，不会静默替换旧配置
- 勾选「WSL同步」后同时写入 WSL 侧对应路径；未在 WSL 中安装对应 agent 时禁用勾选

## 缺省值与字段映射

- 配置未写 `timeout` / `timeoutMs` 时显示默认 `180000` ms，未修改时不写回
- DSH 的 `retryPolicy.mode` 缺省显示 `normal`
- pi 的 `compat.requiresReasoningContentOnAssistantMessages` 与 omp 的 `compat.requiresReasoningContentForAllAssistantTurns` 相互映射；加载 opencode / DSH 或新建时默认不勾选

## 配置文件格式参考

### opencode

工具读取 / 写入 `opencode.json`：

```jsonc
{
  "agent": {
    "my-agent": {
      "mode": "subagent",
      "description": "我的子代理",
      "model": "openai/gpt-4o",
      "variant": "",
      "temperature": 0.7,
      "color": "gold",
      "system": "系统提示词"
    }
  },
  "provider": {
    "openai": {
      "npm": "@ai-sdk/openai",
      "options": {
        "baseURL": "https://api.openai.com/v1",
        "apiKey": "sk-...",
        "timeout": 180000
      },
      "models": {
        "gpt-4o": {
          "name": "GPT-4o",
          "reasoning": false,
          "tool_call": true,
          "limit": { "context": 128000, "output": 4096 },
          "modalities": { "input": ["text"], "output": ["text"] },
          "variants": { "high": { "reasoningEffort": "high" } }
        }
      }
    }
  }
}
```

### pi

工具读取 / 写入 `~/.pi/agent/models.json`：

```json
{
  "providers": {
    "openai": {
      "baseUrl": "https://api.openai.com/v1",
      "apiKey": "sk-...",
      "api": "openai-completions",
      "models": [
        {
          "id": "gpt-4o",
          "name": "GPT-4o",
          "reasoning": false,
          "input": ["text"],
          "contextWindow": 128000,
          "maxTokens": 4096
        }
      ]
    }
  }
}
```

### oh-my-pi

工具读取 / 写入 `~/.omp/agent/models.yml`（本地优先，本地不可用回落 WSL），YAML 格式，结构与 pi 同族：

```yaml
providers:
  my-gateway:
    baseUrl: https://gateway.example.com/v1
    api: openai-completions
    apiKey: sk-...
    authHeader: true            # 注入 Authorization: Bearer
    headers:                    # 原样保留
      X-Team: platform
    models:
    - id: m1
      name: Model One
      reasoning: true
      input: [text, image]
      contextWindow: 200000
      maxTokens: 16384
      thinking:
        mode: effort
        efforts: [medium, high, xhigh, max]
```

### DeepSeek Harness（DSH）

工具读取 / 写入 `~/.dsh/settings.yaml`，只管理 `llm-pi-ai.providers`，其余顶层配置（`ui`、`conversation`、`agent-default-model`、插件设置等）原样保留：

```yaml
llm-pi-ai:
  providers:
    sensenova:
      apiKeyEnv: SENSENOVA_API_KEY   # 凭据引用名，存于主配置
      api: openai-completions
      baseURL: https://api.sensenova.cn/v1
      timeoutMs: 180000
      retryPolicy:
        mode: normal
        maxRetries: 3
      models:
        - id: deepseek-v4-flash
          name: DeepSeek V4 Flash
          contextWindow: 131072
          maxTokens: 8192
          input: [text]
          reasoningEfforts:
            medium: medium
```

## 字段对照

| 字段 | opencode | pi | oh-my-pi |
| ------ | ---------- | ---------- | ---------- |
| Provider key | `provider.{name}` | `providers.{name}` | `providers.{name}` |
| Base URL | `options.baseURL` | `baseUrl` | `baseUrl` |
| API Key | `options.apiKey` | `apiKey` | `apiKey`（环境变量名或字面量） |
| 模型存储 | Map（key = model id） | Array（含 id 字段） | Array（含 id 字段） |
| 上下文长度 | `limit.context` | `contextWindow` | `contextWindow` |
| 输出限制 | `limit.output` | `maxTokens` | `maxTokens` |
| 输入模态 | `modalities.input` | `input` | `input` |
| API 类型 | `npm` | `api` | `api`（9 种枚举） |
| 推理档位 | `variants` | `thinkingLevelMap` | `thinking: {mode, efforts, effortMap}` |
| 工具调用 | `tool_call` | 不支持 | 不支持 |
| Agent 定义 | `agent` | 不支持 | 不支持 |
| 扩展字段 | 顶层字段保留 | 顶层字段保留 | provider / model 级字段保留 |

## 注意事项

- `baseURL` 末尾 `/v1` 的归一化按目标 agent 的客户端行为决定，**读入与写出都做**：
  pi / oh-my-pi / DSH 的 `anthropic-messages` **去掉**末尾 `/v1`（这三家客户端都自己拼 `/v1/messages`，
  base 里再带 `/v1` 会请求成 `/v1/v1/messages`）；opencode 的 `@ai-sdk/anthropic` 相反，baseURL
  **必须带** `/v1`（客户端只追加 `/messages`），读入时缺了就补上、写出时也保证带上；其他 api 一律不动
- provider / model 只保存各自支持的字段，方言字段不会互相泄漏
- oh-my-pi 的 `apiKey` 为「环境变量名或字面量」语义；推理档位保存为官方 `thinking` 块
- 保存 YAML 时文件注释不会保留，输出为标准块风格
- DSH 的实际密钥保存在同级 `.credentials.yaml` 的 `refs` 下，加载时自动读取，保存时写回；凭据文件中的其他字段原样保留

## 平台与安全

- 当前**仅支持 Windows**
- 配置文件中的 `apiKey` 为**明文**，DSH 的 `.credentials.yaml` 同样为明文，请勿提交到公开仓库
