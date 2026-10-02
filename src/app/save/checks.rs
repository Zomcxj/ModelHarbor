use crate::app::App;
use crate::credentials;
use crate::format::ConfigFormat;
use crate::util::parse_number_text;
use std::collections::HashSet;

impl App {
    /// 将已加载的公共 provider 凭据投影到 DSH 专属字段。
    /// 只在进入 DSH 页面时执行一次，避免用户在页面内主动清空后被立即回填。
    pub(in crate::app) fn project_dsh_credentials(&mut self) {
        for provider in &mut self.providers {
            if provider.api_key_env.trim().is_empty() {
                provider.api_key_env = credentials::default_env_name(&provider.key);
            }
        }
    }

    /// 页面切换时保持 provider 密钥一致：DSH 页使用 api_key_secret（对应
    /// .credentials.yaml 的 refs），其他页面使用 api_key。切换时把非空值
    /// 同步到目标页字段；若用户在 DSH 页明确清空过密钥（原本有、当前空），
    /// 不再用其他页面的旧值覆盖。
    pub(in crate::app) fn sync_provider_secrets(&mut self, target: ConfigFormat) {
        for provider in &mut self.providers {
            if target == ConfigFormat::DeepSeekHarness {
                let cleared_on_dsh = !provider.original_api_key_secret.is_empty()
                    && provider.api_key_secret.is_empty();
                if !provider.api_key.trim().is_empty() && !cleared_on_dsh {
                    provider.api_key_secret = provider.api_key.clone();
                }
            } else {
                let cleared_on_dsh = !provider.original_api_key_secret.is_empty()
                    && provider.api_key_secret.is_empty();
                if cleared_on_dsh {
                    // 与 DSH 方向对称：在 DSH 页明确清空过的密钥（原本有、当前空）
                    // 不再用旧值填充其他页面，避免已清空的密钥被写回 opencode 等配置。
                    provider.api_key = String::new();
                } else if !provider.api_key_secret.trim().is_empty() {
                    provider.api_key = provider.api_key_secret.clone();
                }
            }
        }
    }

    /// 统计非法数字字段数（非空且解析失败），保存后提示用户它们被忽略。
    pub(in crate::app) fn count_invalid_numeric_fields(&self) -> usize {
        fn bad(s: &str) -> bool {
            !s.trim().is_empty() && parse_number_text(s).is_none()
        }
        let mut n = 0;
        for a in &self.agents {
            if bad(&a.temperature) {
                n += 1;
            }
        }
        for p in &self.providers {
            if bad(&p.timeout) {
                n += 1;
            }
            for m in &p.models {
                if bad(&m.context) || bad(&m.output) {
                    n += 1;
                }
            }
        }
        n
    }

    /// 校验 agent / provider / model key 唯一性，返回首个冲突描述。
    pub(in crate::app) fn find_duplicate_keys(&self) -> Option<String> {
        let mut seen = HashSet::new();
        for a in &self.agents {
            let k = a.key.trim();
            if !k.is_empty() && !seen.insert(k.to_string()) {
                return Some(format!("agent \"{}\"", k));
            }
        }
        let mut seen_p = HashSet::new();
        for p in &self.providers {
            let k = p.key.trim();
            if k.is_empty() {
                continue;
            }
            if !seen_p.insert(k.to_string()) {
                return Some(format!("provider \"{}\"", k));
            }
            let mut seen_m = HashSet::new();
            for m in &p.models {
                let mk = m.id.trim();
                if !mk.is_empty() && !seen_m.insert(mk.to_string()) {
                    return Some(format!("provider \"{}\" 的 model \"{}\"", k, mk));
                }
            }
        }
        None
    }
}
