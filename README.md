# ModelHarbor

可视化编辑 [`opencode`](https://opencode.ai)、[`pi`](https://github.com/earendil-works/pi)、[`oh-my-pi`](https://github.com/can1357/oh-my-pi)（omp）与 [`DeepSeek Harness`](https://github.com/amorvincit-omnia/llm-pi-ai)（DSH）配置文件的桌面 GUI 工具。

Rust + egui 构建，单文件可执行程序，无需安装运行时。

## 功能特性

- **多页面编辑**：顶栏图标切换 opencode / pi / omp / DSH 页面；加载任意一份配置后各页共享同一份数据，修改即时同步（provider / model 顺序亦跨页同步）
- **多格式互转**：任意加载、任意保存；跨格式写入只接管 provider / providers（条目与顺序以界面为准；opencode 的 agent 仅在界面持有 agents 数据时接管，否则目标文件已有 agents 保留），目标文件其余配置保留，覆盖前自动备份为 `.bak`
- **方言表单**：各页按自身格式显示字段与枚举（api 下拉、推理档位、字段标签），没有的字段不占位；协议（npm / api）四页共用一份数据，下拉首项「(空)」= 不指定协议，跨页同步
- **卡片式管理**：Agents / Providers 增删改复制、拖拽排序、折叠展开；模型参数与推理档位编辑
- **获取模型**：一键从提供商的模型列表接口拉取模型，多列勾选，勾选未配置的模型即新增
- **延迟 / 连通性测试**：Providers 标题行一键测试全部厂商连通性；模型延迟为**单模型手动探测**（每个模型行右侧「测试」按钮，一次只测一个），带防封号保护：检测到系统代理 / VPN 时禁用、同厂商任意两次探测间隔 ≥5 秒（跨厂商互不牽连、可并行）、同厂商串行、**全协议流式测首字延迟**（读完 SSE 再断开）、问句轮换不固定 `ping`、UA 按协议伪装白名单客户端（`claude-cli` / `opencode`）；端点与鉴权按所选协议构造（超时 10 秒），结果显示在厂商与模型卡片上，失败显示错误码
- **配置预览 / 编辑**：右侧面板实时显示待保存内容，JSON / YAML 语法高亮，可直接编辑（改动实时应用到表单并自动保存），支持 Ctrl+F 查找、切页重置草稿、一键「重新生成」、拖动分隔条调整宽度
- **凭据处理**：各格式按自身方式读写密钥（DSH 为 `apiKeyEnv` 引用 + 同级 `.credentials.yaml` 实际密钥），加载自动读取、跨页同步、保存写回
- **分页保存与 WSL 同步**：每页独立保存按钮与写入路径，默认写 Windows 本地；勾选「WSL同步」且对应 agent 已在 WSL 中安装时同步写入
- **缺省值**：未写 `timeout` / `timeoutMs` 时显示 `180000` ms、`retryPolicy.mode` 显示 `normal`；未修改不写回，不污染配置
- **其他**：自动格式检测、拖拽导入、pretty / compact 两种保存格式、五种主题、中文界面、区块标题吸顶、顶栏纯图标（悬停显示名称）

## 构建运行

```bash
cargo build --release
```

产物为单文件可执行程序：`target/release/ModelHarbor.exe`

## 更多细节

字段对照表、各格式配置示例、注意事项与平台安全说明，见 **[docs/DETAILS.md](docs/DETAILS.md)**。

## 许可证

见 [LICENSE](LICENSE)。
