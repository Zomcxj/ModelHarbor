//! The pre-probe network risk guard: blocks model latency probes when a system proxy or VPN / TUN exit is found.
//!
//! **Can detect**:
//! - Windows system proxy (registry `Internet Settings`: `ProxyEnable` / `AutoConfigURL`)
//! - VPN / TUN adapters: `IfType` PPP(23) / TUNNEL(131), or the adapter name / description hitting VPN keywords,
//!   counting only adapters in the `Up` state
//!
//! **Cannot detect**: transparent proxies, router / gateway-level proxies, TUN implementations rerouting below the stack.
//!
//! This tool's HTTP client (ureq) ignores system proxy settings; probe requests always connect directly.

/// An adapter's identifying information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    /// The connection name (e.g. `Ethernet`, `Clash`).
    pub name: String,
    /// The description (e.g. `Wintun Userspace Tunnel`).
    pub description: String,
    /// `IfType` (IANA ifType): 23 = PPP, 131 = TUNNEL.
    pub if_type: u32,
    /// Whether it's in the `Up` state.
    pub up: bool,
}

/// `IfType` (IANA ifType) values strongly associated with VPN / tunnels.
const IF_TYPE_PPP: u32 = 23;
const IF_TYPE_TUNNEL: u32 = 131;

/// VPN / TUN adapter-name keywords (lowercase-matched against "connection name + description").
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

/// The prefix of the probe-blocked notice (reused by the UI).
pub const BLOCK_PREFIX: &str = "模型延迟测试已禁用";

/// Judge a proxy exit from the system-proxy registry values.
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

/// Judge a VPN / TUN exit from the adapter list.
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

/// The connection name or description hits a VPN keyword.
fn is_vpn_name(adapter: &Adapter) -> bool {
    let haystack = format!("{} {}", adapter.name, adapter.description).to_lowercase();
    VPN_KEYWORDS
        .iter()
        .any(|keyword| haystack.contains(keyword))
}

/// The pre-probe network risk check: returns why probing is blocked (`None` = no proxy exit found).
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

    /// The registry path holding system proxy settings.
    const INTERNET_SETTINGS: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";

    /// Read system proxy settings; returns why probing is blocked.
    pub(super) fn system_proxy_reason() -> Option<String> {
        let subkey = to_wide(INTERNET_SETTINGS);
        let mut hkey: HKEY = std::ptr::null_mut();
        // SAFETY: valid wide-string and output pointers passed in; returns immediately on failure.
        let opened =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) };
        if opened != ERROR_SUCCESS {
            return None;
        }
        let enable = query_dword(hkey, "ProxyEnable");
        let server = query_string(hkey, "ProxyServer");
        let pac = query_string(hkey, "AutoConfigURL");
        // SAFETY: hkey comes from the successful RegOpenKeyExW above.
        unsafe { RegCloseKey(hkey) };
        proxy_reason(enable, server.as_deref(), pac.as_deref())
    }

    /// Enumerate this machine's adapters (only the platform-visible name / description / IfType / state).
    pub(super) fn adapters() -> Vec<Adapter> {
        let mut size: u32 = 16 * 1024;
        let mut buf = vec![0u8; size as usize];
        // SAFETY: buffer and length per the API contract; on ERROR_BUFFER_OVERFLOW retry with the new length.
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
            // SAFETY: same as above, with the buffer reallocated to the API-reported required length.
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
            // SAFETY: on success, buf holds a NUL-terminated linked list.
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

    /// `AF_UNSPEC`: no address-family restriction.
    const AF_UNSPEC: u32 = 0;

    fn query_dword(hkey: HKEY, name: &str) -> Option<u32> {
        let name = to_wide(name);
        let mut kind: REG_VALUE_TYPE = 0;
        let mut buf = [0u8; 4];
        let mut len = buf.len() as u32;
        // SAFETY: the name and buffer pointers are valid; the length matches the buffer.
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
        // SAFETY: take the length first (lpdata a null pointer), then allocate the buffer by it and read.
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
        // SAFETY: the buffer length matches what the API reported.
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
        // REG_SZ's byte length is always even.
        let (pairs, _) = buf.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
        Some(String::from_utf16_lossy(&units[..end]))
    }

    fn wide_ptr_to_string(ptr: windows_sys::core::PWSTR) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        // SAFETY: the pointer comes from GetAdaptersAddresses' list, pointing at a NUL-terminated wide string.
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
        assert!(proxy_reason(Some(0), Some("127.0.0.1:7890"), Some("")).is_none());
        assert!(proxy_reason(Some(0), Some("  "), None).is_none());
        let reason = proxy_reason(Some(1), Some("127.0.0.1:7890"), None).unwrap();
        assert!(reason.contains("127.0.0.1:7890"), "{}", reason);
        assert!(proxy_reason(Some(1), Some("   "), None)
            .unwrap()
            .contains("系统代理已开启"));
        assert!(
            proxy_reason(Some(0), None, Some("http://pac.local/proxy.pac"))
                .unwrap()
                .contains("PAC")
        );
    }

    #[test]
    fn vpn_reason_matches_type_and_names_but_ignores_down_adapters() {
        assert!(vpn_reason(&[]).is_none());
        assert!(vpn_reason(&[
            adapter("以太网", "Realtek PCIe GbE Family Controller", 6, true),
            adapter("WLAN", "Intel(R) Wi-Fi 6 AX201 160MHz", 71, true),
        ])
        .is_none());
        // A TUN adapter with Wintun in the description
        let reason = vpn_reason(&[adapter("Clash", "Wintun Userspace Tunnel", 6, true)]).unwrap();
        assert!(reason.contains("Clash"), "{}", reason);
        assert!(vpn_reason(&[adapter("WireGuard Tunnel", "WireGuard", 6, true)]).is_some());
        assert!(vpn_reason(&[adapter("OpenVPN TAP-Windows6", "", 6, true)]).is_some());
        assert!(vpn_reason(&[adapter("Tailscale", "Tailscale Tunnel", 6, true)]).is_some());
        // IfType hit (tunnel / PPP)
        assert!(vpn_reason(&[adapter("本地连接* 12", "", IF_TYPE_TUNNEL, true)]).is_some());
        assert!(vpn_reason(&[adapter("宽带连接", "", IF_TYPE_PPP, true)]).is_some());
        // A down adapter doesn't count
        assert!(vpn_reason(&[adapter("Clash", "Wintun Userspace Tunnel", 6, false)]).is_none());
        assert!(vpn_reason(&[adapter("WireGuard Tunnel", "", IF_TYPE_TUNNEL, false)]).is_none());
    }

    #[test]
    fn detect_returns_something_or_nothing_without_panicking() {
        // Only verify the real detection path doesn't panic (the result depends on this machine)
        let _ = detect();
    }
}
