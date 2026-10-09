use super::*;

impl super::App {
    /// 按 provider 的 api 类型构造模型列表接口地址。
    pub(in crate::app) fn models_url(base_url: &str, api: &str) -> String {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return String::new();
        }
        match api_wire(api) {
            // Anthropic 的模型接口固定为 /v1/models。
            ApiWire::AnthropicMessages => {
                if base.ends_with("/v1") {
                    format!("{}/models", base)
                } else {
                    format!("{}/v1/models", base)
                }
            }
            // Vertex 的模型列表挂在 publishers/google 下。
            ApiWire::GoogleVertex => format!("{}/publishers/google/models", base),
            _ => format!("{}/models", base),
        }
    }

    /// 启动 provider 级延迟测试（后台线程，结果经通道回传）。
    /// 接收 `&mut HashMap` 而非 `&mut self`，以便与 `providers[idx]` 借用共存。
    pub(in crate::app) fn start_provider_latency(
        latency: &mut HashMap<String, LatencyState>,
        key: &str,
        base_url: &str,
        secret: &str,
        api: &str,
    ) {
        let url = Self::models_url(base_url, api);
        let secret = secret.trim().to_string();
        let api = api.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(measure_provider_latency(&url, &secret, &api));
        });
        let state = latency.entry(key.to_string()).or_default();
        state.provider = None;
        state.provider_rx = Some(rx);
    }

    /// 单模型探测的统一入口：门控 → 选问句 → 启动后台线程 → 返回状态栏消息。
    ///
    /// 写成关联函数（不借 `&mut self`）以便在 provider 卡片内部调用。
    #[allow(clippy::too_many_arguments)]
    pub(in crate::app) fn run_model_probe(
        probe: &mut ProbeGate,
        latency: &mut HashMap<String, LatencyState>,
        net_guard: Option<&str>,
        provider_key: &str,
        model_id: &str,
        now: f64,
        base_url: &str,
        secret: &str,
        api: &str,
    ) -> String {
        match probe.state(provider_key, now, net_guard) {
            ProbeGateState::Ready => {
                let question = probe.next_question(provider_key);
                probe.start(provider_key, now);
                Self::start_model_latency(
                    latency,
                    provider_key,
                    base_url,
                    secret,
                    api,
                    model_id,
                    question,
                );
                format!("正在测试 {} 的 {} 延迟…", provider_key, model_id)
            }
            ProbeGateState::Cooling(left) => {
                format!("节流中：{} 秒后可再测", left.ceil() as u64)
            }
            ProbeGateState::Busy => "上一个延迟测试尚未结束（一次只测一个模型）".to_string(),
            ProbeGateState::NetBlocked(reason) => {
                format!("{}：{}", crate::netguard::BLOCK_PREFIX, reason)
            }
        }
    }

    /// 启动单个模型的延迟探测（后台线程，结果经通道回传）。
    ///
    /// 一次只测一个，节流与串行由 [`ProbeGate`] 把关。
    pub(in crate::app) fn start_model_latency(
        latency: &mut HashMap<String, LatencyState>,
        key: &str,
        base_url: &str,
        secret: &str,
        api: &str,
        model: &str,
        question: &str,
    ) {
        let base = base_url.to_string();
        let secret = secret.trim().to_string();
        let api = api.to_string();
        let model = model.trim().to_string();
        let question = question.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker_model = model.clone();
        std::thread::spawn(move || {
            let result = measure_model_latency(&base, &secret, &api, &worker_model, &question);
            let _ = tx.send((worker_model, result));
        });
        let state = latency.entry(key.to_string()).or_default();
        state.done = 0;
        state.total = 1;
        state.pending.clear();
        state.pending.insert(model);
        state.model_rx = Some(rx);
    }

    /// 每帧轮询延迟测试结果，并更新状态栏。
    pub(in crate::app) fn poll_latency(&mut self) {
        let mut notices: Vec<String> = Vec::new();
        for (key, state) in self.latency.iter_mut() {
            if let Some(rx) = &state.provider_rx {
                match rx.try_recv() {
                    Ok(result) => {
                        // 失败原因已就地显示在厂商行（红字 + 悬停详情），不推送到底部状态栏。
                        if let Ok(ms) = &result {
                            notices.push(format!("provider 延迟测试完成：{} ms", ms));
                        }
                        state.provider = Some(result);
                        state.provider_rx = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // 测量线程异常退出时收不到结果：终止等待，不在底部报错。
                        state.provider_rx = None;
                    }
                }
            }
            if let Some(rx) = &state.model_rx {
                loop {
                    match rx.try_recv() {
                        Ok((id, result)) => {
                            state.pending.remove(&id);
                            state.models.insert(id, result);
                            state.done += 1;
                            // 单模型探测：拿到结果即释放该 provider 的串行位。
                            self.probe.finish(key);
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            state.model_rx = None;
                            state.total = state.done;
                            // 单模型探测：total 恒为 1，只区分「已回传结果」与「线程异常退出」。
                            let unfinished = !state.pending.is_empty();
                            state.pending.clear();
                            notices.push(if unfinished {
                                "模型延迟测试中断（线程异常退出）".to_string()
                            } else {
                                "模型延迟测试完成".to_string()
                            });
                            break;
                        }
                    }
                }
            }
        }
        for msg in notices {
            self.status = msg;
        }
    }

    /// 后台拉取某个后端的内置网关免费模型列表（已有请求在飞时不重复发起）。
    /// 请求公共模型库，不需要 baseURL / API Key。
    pub(in crate::app) fn start_free_models_fetch(&mut self, format: crate::format::ConfigFormat) {
        if crate::opencode_models::source_for(format).is_none() {
            return;
        }
        let state = self.free_models.entry(format).or_default();
        if state.fetching() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::opencode_models::fetch_remote(format));
        });
        state.rx = Some(rx);
    }

    /// 每帧轮询各后端的免费模型拉取结果：成功则刷新列表并落盘缓存。
    /// 拉取失败不清空已有列表，只在 Agents 区块旁提示。
    pub(in crate::app) fn poll_free_models(&mut self) {
        for (format, state) in self.free_models.iter_mut() {
            let Some(rx) = &state.rx else {
                continue;
            };
            match rx.try_recv() {
                Ok(Ok(models)) => {
                    state.rx = None;
                    state.error = None;
                    if !models.is_empty() {
                        // 缓存写失败不影响本次使用。
                        let _ = crate::opencode_models::save_cache(*format, &models);
                        state.models = models;
                    }
                }
                Ok(Err(err)) => {
                    state.rx = None;
                    state.error = Some(err);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                // 线程异常退出：终止等待。
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    state.rx = None;
                }
            }
        }
    }
}
