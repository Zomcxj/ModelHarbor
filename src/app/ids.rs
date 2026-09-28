use super::*;

impl super::App {
    /// 某一页「相对默认路径」的覆盖值：与默认相同（或没改过）返回空串，
    /// 这样 prefs 里只留真正手动指定过的路径，默认路径永远跟着自动探测走。
    ///
    /// 比较基准用**解析后的默认路径**（`resolve_local_path`）：自动落到 `.jsonc`
    /// 变体上只是探测结果，不是用户的选择，不该被当成手动覆盖记进 settings.json——
    /// 否则 CLI 之后把文件改名成 `.json`，这条覆盖就指向一个不存在的路径了。
    pub(in crate::app) fn path_override(&self, format: ConfigFormat) -> String {
        let current = self.config_paths.local_path(format);
        let default =
            backends::resolve_local_path(format, &ConfigPaths::default_local_path(format));
        if current.trim() == default.trim() {
            String::new()
        } else {
            current
        }
    }

    /// 当前配置文件身份。各页面共享同一份加载数据，故只按路径分区，不按页面分区。
    pub(in crate::app) fn config_id(&self) -> String {
        let path = if self.loaded_path.trim().is_empty() {
            &self.config_path
        } else {
            &self.loaded_path
        };
        crate::prefs::config_identity(path)
    }

    /// 卡片折叠状态的持久化键（按配置与类别区分）。
    pub(in crate::app) fn card_id(&self, kind: &str, key: &str) -> String {
        crate::prefs::collapsed_id(&self.config_id(), kind, key)
    }

    /// 某 baseUrl 所属站点的面板令牌（没设置则空串）。
    ///
    /// 键是规范化 origin：同一站点的多个 provider 共用一份令牌。
    pub(in crate::app) fn station_pat(&self, base_url: &str) -> String {
        let key = crate::tokens::station_key(base_url);
        self.tokens.get(&key).to_string()
    }

    /// 某 baseUrl 所属站点的用户 ID（没设置则空串；空串 = 不发 `New-Api-User`）。
    pub(in crate::app) fn station_user_id(&self, base_url: &str) -> String {
        let key = crate::tokens::station_key(base_url);
        self.tokens.user_id(&key).to_string()
    }
}
