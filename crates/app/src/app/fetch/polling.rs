use super::*;

impl super::App {
    /// Build the model-list endpoint's address per the provider's api type.
    pub(in crate::app) fn models_url(base_url: &str, api: &str) -> String {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return String::new();
        }
        match api_wire(api) {
            // Anthropic's model endpoint is fixed at /v1/models.
            ApiWire::AnthropicMessages => {
                if base.ends_with("/v1") {
                    format!("{}/models", base)
                } else {
                    format!("{}/v1/models", base)
                }
            }
            // Vertex's model list hangs under publishers/google.
            ApiWire::GoogleVertex => format!("{}/publishers/google/models", base),
            _ => format!("{}/models", base),
        }
    }

    /// Start a provider-level latency test (background thread; results return over a channel).
    /// Takes `&mut HashMap` rather than `&mut self` so it can coexist with the `providers[idx]` borrow.
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

    /// The unified entry for single-model probes: gate → pick question → start the background thread → return the status note.
    ///
    /// An associated function (no `&mut self` borrow) so provider cards can call it internally.
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

    /// Start one model's latency probe (background thread; results return over a channel).
    ///
    /// One at a time, with [`ProbeGate`] handling throttle and serialization.
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

    /// Poll latency-test results each frame, updating the status bar.
    pub(in crate::app) fn poll_latency(&mut self) {
        let mut notices: Vec<String> = Vec::new();
        for (key, state) in self.latency.iter_mut() {
            if let Some(rx) = &state.provider_rx {
                match rx.try_recv() {
                    Ok(result) => {
                        // Failure reasons already display in place on the vendor row (red text + hover details), never pushed to the bottom status bar.
                        if let Ok(ms) = &result {
                            notices.push(format!("provider 延迟测试完成：{} ms", ms));
                        }
                        state.provider = Some(result);
                        state.provider_rx = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // An abnormally exited measurement thread never delivers results: end the wait, no bottom-bar error.
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
                            // Single-model probes: release the provider's serialization slot as soon as a result arrives.
                            self.probe.finish(key);
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            state.model_rx = None;
                            state.total = state.done;
                            // Single-model probes: total is always 1, distinguishing only "result delivered" from "thread died".
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

    /// Fetch a backend's built-in gateway free-model list in the background (no repeat while a request is in flight).
    /// Hits the public model library — no baseURL / API Key needed.
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

    /// Poll each backend's free-model fetch results each frame: on success, refresh the list and persist the cache.
    /// Failures don't clear the existing list — only a note beside the Agents block.
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
                        // A failed cache write doesn't affect this round's use.
                        let _ = crate::opencode_models::save_cache(*format, &models);
                        state.models = models;
                    }
                }
                Ok(Err(err)) => {
                    state.rx = None;
                    state.error = Some(err);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                // The thread died abnormally: end the wait.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    state.rx = None;
                }
            }
        }
    }
}
