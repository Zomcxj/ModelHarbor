//! 探测前的网络风险守卫：发现系统代理或 VPN / TUN 出口时禁止发起模型延迟探测。
//!
//! 中转站（API 中转）普遍带「多 IP 检测 / 测活封号」风控：同一个 key 在短时间内
//! 从多个出口 IP 反复发探测请求，很容易被判为滥用并封号。本模块在探测前识别本机
//! 的常见代理出口，命中即禁止探测并说明原因。
//!
//! **能识别**：
//! - Windows 系统代理（注册表 `Internet Settings`：`ProxyEnable` / `AutoConfigURL`）
//! - VPN / TUN 网卡：`IfType` 为 PPP(23) / TUNNEL(131)，或网卡名 / 描述命中 VPN 关键词，
//!   且只统计处于 `Up` 状态的网卡（未启用的 VPN 网卡不阻止探测）
//!
//! **识别不了**：透明代理、路由器 / 网关级代理、在协议栈下方改道的 TUN 实现。
//! 这类改道对用户态进程完全不可见，工具无法检测——只能靠用户按文档自行判断。
//!
//! 另外：本工具的 HTTP 客户端（ureq）**不使用**系统代理设置，探测请求始终直连，
//! 因此「开着 Clash 的系统代理」不会让探测走代理出口；守卫的作用是避免在
//! 出口 IP 已经变化（VPN 生效）时继续探测。

/// 网卡的识别信息（与平台无关，便于单测）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    /// 连接名（如 `以太网`、`Clash`）。
    pub name: String,
    /// 描述（如 `Wintun Userspace Tunnel`）。
    pub description: String,
    /// `IfType`（IANA ifType）：23 = PPP，131 = TUNNEL。
    pub if_type: u32,
    /// 是否处于 `Up` 状态。
    pub up: bool,
}

/// `IfType`（IANA ifType）中与 VPN / 隧道强相关的取值。
const IF_TYPE_PPP: u32 = 23;
const IF_TYPE_TUNNEL: u32 = 131;

/// VPN / TUN 网卡名关键词（对「连接名 + 描述」小写匹配，作为 IfType 判定的补充）。
const VPN_KEYWORDS: [&str; 16] = [
    "vpn",
    "wireguard",
    "openvpn",
    "tap-windows",
    "wintun",
    "tun",
    "clash",
    "v2ray",
    "xray",
    "sing-box",
    "singbox",
    "shadowsocks",
    "trojan",
    "tailscale",
    "zerotier",
    "proxy",
];

/// 探测被禁止的提示前缀（供 UI 复用）。
pub const BLOCK_PREFIX: &str = "模型延迟测试已禁用";

/// 由系统代理注册表值判断是否处于代理出口（纯函数，便于单测）。
pub fn proxy_reason(
    proxy_enable: Option<u32>,
    proxy_server: Option<&str>,
    auto_config_url: Option<&str>,
) -> Option<String> {
    if auto_config_url.is_some_and(|s| !s.trim().is_empty()) {
        return Some("系统已启用自动配置脚本（PAC）代理".to_string());
    }
    if proxy_enable == Some(1) {
        return Some(
            match proxy_server.map(str::trim).filter(|s| !s.is_empty()) {
                Some(server) => format!("系统代理已开启（{}）", server),
                None => "系统代理已开启".to_string(),
            },
        );
    }
    None
}

/// 由网卡列表判断是否存在 VPN / TUN 出口（纯函数，便于单测）。
pub fn vpn_reason(adapters: &[Adapter]) -> Option<String> {
    let hit = adapters.iter().find(|a| {
        a.up && (a.if_type == IF_TYPE_PPP || a.if_type == IF_TYPE_TUNNEL || is_vpn_name(a))
    })?;
    let name = hit.name.trim();
    let label = if name.is_empty() {
        hit.description.trim()
    } else {
        name
    };
    let label = if label.is_empty() {
        "未知网卡"
    } else {
        label
    };
    Some(format!("检测到 VPN / 隧道网卡（{}）", label))
}

/// 连接名或描述命中 VPN 关键词。
fn is_vpn_name(adapter: &Adapter) -> bool {
    let haystack = format!("{} {}", adapter.name, adapter.description).to_lowercase();
    VPN_KEYWORDS
        .iter()
        .any(|keyword| haystack.contains(keyword))
}

/// 探测前的网络风险检查：返回禁止探测的原因（`None` 表示未发现代理出口）。
pub fn detect() -> Option<String> {
    #[cfg(windows)]
    {
        if let Some(reason) = system_proxy_reason() {
            return Some(reason);
        }
        vpn_reason(&adapters())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
mod platform {
    use super::{proxy_reason, Adapter};
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
        REG_SZ, REG_VALUE_TYPE,
    };

    /// 系统代理设置所在的注册表路径。
    const INTERNET_SETTINGS: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";

    /// 读系统代理设置，返回禁止探测的原因。
    pub(super) fn system_proxy_reason() -> Option<String> {
        let subkey = to_wide(INTERNET_SETTINGS);
        let mut hkey: HKEY = std::ptr::null_mut();
        // SAFETY: 传入合法的宽字符串指针与输出指针；失败立即返回。
        let opened =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) };
        if opened != ERROR_SUCCESS {
            return None;
        }
        let enable = query_dword(hkey, "ProxyEnable");
        let server = query_string(hkey, "ProxyServer");
        let pac = query_string(hkey, "AutoConfigURL");
        // SAFETY: hkey 来自上面成功的 RegOpenKeyExW。
        unsafe { RegCloseKey(hkey) };
        proxy_reason(enable, server.as_deref(), pac.as_deref())
    }

    /// 枚举本机网卡（只取平台可见的名称 / 描述 / IfType / 状态）。
    pub(super) fn adapters() -> Vec<Adapter> {
        let mut size: u32 = 16 * 1024;
        let mut buf = vec![0u8; size as usize];
        // SAFETY: 缓冲区与长度按 API 约定传入；返回 ERROR_BUFFER_OVERFLOW 时按新长度重试。
        let mut rc = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC,
                0,
                std::ptr::null(),
                buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut size,
            )
        };
        if rc == ERROR_BUFFER_OVERFLOW {
            buf = vec![0u8; size as usize];
            // SAFETY: 同上，缓冲区已按 API 返回的所需长度重新分配。
            rc = unsafe {
                GetAdaptersAddresses(
                    AF_UNSPEC,
                    0,
                    std::ptr::null(),
                    buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                    &mut size,
                )
            };
        }
        if rc != ERROR_SUCCESS {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cursor = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !cursor.is_null() {
            // SAFETY: 成功返回时 buf 内是一条以空指针结尾的链表。
            let adapter = unsafe { &*cursor };
            out.push(Adapter {
                name: wide_ptr_to_string(adapter.FriendlyName),
                description: wide_ptr_to_string(adapter.Description),
                if_type: adapter.IfType,
                up: adapter.OperStatus == IfOperStatusUp,
            });
            cursor = adapter.Next;
        }
        out
    }

    /// `AF_UNSPEC`：不限制地址族（避免额外依赖 WinSock 常量导入）。
    const AF_UNSPEC: u32 = 0;

    fn query_dword(hkey: HKEY, name: &str) -> Option<u32> {
        let name = to_wide(name);
        let mut kind: REG_VALUE_TYPE = 0;
        let mut buf = [0u8; 4];
        let mut len = buf.len() as u32;
        // SAFETY: 名称与缓冲区指针有效，长度与缓冲区一致。
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                buf.as_mut_ptr(),
                &mut len,
            )
        };
        if rc != ERROR_SUCCESS || kind != REG_DWORD || len < 4 {
            return None;
        }
        Some(u32::from_le_bytes(buf))
    }

    fn query_string(hkey: HKEY, name: &str) -> Option<String> {
        let name = to_wide(name);
        let mut kind: REG_VALUE_TYPE = 0;
        let mut len: u32 = 0;
        // SAFETY: 先取长度（lpdata 为空指针），再按长度分配缓冲区取值。
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut len,
            )
        };
        if rc != ERROR_SUCCESS || kind != REG_SZ || len < 2 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let mut written = len;
        // SAFETY: 缓冲区长度与 API 报告的长度一致。
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                buf.as_mut_ptr(),
                &mut written,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let units: Vec<u16> = buf
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
        Some(String::from_utf16_lossy(&units[..end]))
    }

    fn wide_ptr_to_string(ptr: windows_sys::core::PWSTR) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        // SAFETY: 指针来自 GetAdaptersAddresses 的链表，指向以 NUL 结尾的宽字符串。
        unsafe {
            while *ptr.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
        }
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
use platform::{adapters, system_proxy_reason};

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str, description: &str, if_type: u32, up: bool) -> Adapter {
        Adapter {
            name: name.to_string(),
            description: description.to_string(),
            if_type,
            up,
        }
    }

    #[test]
    fn proxy_reason_covers_pac_and_explicit_proxy() {
        assert!(proxy_reason(None, None, None).is_none());
        // 关闭的代理、空字符串都不算
        assert!(proxy_reason(Some(0), Some("127.0.0.1:7890"), Some("")).is_none());
        assert!(proxy_reason(Some(0), Some("  "), None).is_none());
        // 开启的代理：带上服务器地址
        let reason = proxy_reason(Some(1), Some("127.0.0.1:7890"), None).unwrap();
        assert!(reason.contains("127.0.0.1:7890"), "{}", reason);
        // 开启但没有服务器地址时也要提示
        assert!(proxy_reason(Some(1), Some("   "), None)
            .unwrap()
            .contains("系统代理已开启"));
        // PAC 优先于 ProxyEnable
        assert!(
            proxy_reason(Some(0), None, Some("http://pac.local/proxy.pac"))
                .unwrap()
                .contains("PAC")
        );
    }

    #[test]
    fn vpn_reason_matches_type_and_names_but_ignores_down_adapters() {
        assert!(vpn_reason(&[]).is_none());
        // 普通有线 / 无线网卡不误判
        assert!(vpn_reason(&[
            adapter("以太网", "Realtek PCIe GbE Family Controller", 6, true),
            adapter("WLAN", "Intel(R) Wi-Fi 6 AX201 160MHz", 71, true),
        ])
        .is_none());
        // 描述里带 Wintun 的 TUN 网卡（Clash 的 TUN 模式）
        let reason = vpn_reason(&[adapter("Clash", "Wintun Userspace Tunnel", 6, true)]).unwrap();
        assert!(reason.contains("Clash"), "{}", reason);
        // 名称命中
        assert!(vpn_reason(&[adapter("WireGuard Tunnel", "WireGuard", 6, true)]).is_some());
        assert!(vpn_reason(&[adapter("OpenVPN TAP-Windows6", "", 6, true)]).is_some());
        assert!(vpn_reason(&[adapter("Tailscale", "Tailscale Tunnel", 6, true)]).is_some());
        // IfType 命中（隧道 / PPP），名称完全无关也能识别
        assert!(vpn_reason(&[adapter("本地连接* 12", "", IF_TYPE_TUNNEL, true)]).is_some());
        assert!(vpn_reason(&[adapter("宽带连接", "", IF_TYPE_PPP, true)]).is_some());
        // 未启用的网卡不阻止探测
        assert!(vpn_reason(&[adapter("Clash", "Wintun Userspace Tunnel", 6, false)]).is_none());
        assert!(vpn_reason(&[adapter("WireGuard Tunnel", "", IF_TYPE_TUNNEL, false)]).is_none());
    }

    #[test]
    fn detect_returns_something_or_nothing_without_panicking() {
        // 只验证真实检测路径不 panic（结果取决于本机是否开着代理 / VPN）
        let _ = detect();
    }
}
