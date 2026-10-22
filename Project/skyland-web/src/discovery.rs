//! 局域网服务端发现 — UDP 广播协议
//! 服务端（server 模式）监听 UDP 3311，收到 SKYLAND_DISCOVER_V1 探测包时
//! 以 JSON 应答 {name, port}（从应答 socket 的 local_addr 可得服务端局域网 IP）。
//! 客户端（Tauri）向 255.255.255.255:3311 广播探测并收集 1.5s 内的应答。
//! standalone 模式只绑 127.0.0.1，不启动应答（局域网内无意义）。
//!
//! ── 品牌改名下的线协议兼容 ──
//! 探测包与应答前缀是**跨版本线协议**：桌面端与服务端可能不在同一台机器、
//! 也不一定同时升级。若只把 MOLINE_* 换成 SKYLAND_*，旧客户端探测新版服务端
//! 会**静默失联**（`parse_reply` 匹配不到前缀即返回 None，全程无任何报错）——
//! 这是最难排查的一类故障。因此采用双向双轨 + **版本回显**：
//!   1. 服务端对两种探测包都应答，且**用客户端所探测的版本前缀作答**
//!      （收到旧包回旧前缀，收到新包回新前缀）——保证任意新旧组合都能解析；
//!   2. 客户端两种探测包都发，任一版本服务端都能被发现；
//!   3. 客户端 `parse_reply` 同时接受两种前缀。
//! 历史常量以 `LEGACY_` 前缀保留，随协议 v1 下线一并移除。

use std::net::UdpSocket;
use std::time::Duration;

pub const DISCOVERY_PORT: u16 = 3311;
pub const PROBE_PACKET: &[u8] = b"SKYLAND_DISCOVER_V1";
pub const REPLY_PREFIX: &str = "SKYLAND_V1 ";
/// 历史协议（moline 时代）——保留以兼容未升级的客户端/服务端。
pub const LEGACY_PROBE_PACKET: &[u8] = b"MOLINE_DISCOVER_V1";
pub const LEGACY_REPLY_PREFIX: &str = "MOLINE_V1 ";

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DiscoveredServer {
    /// 服务端局域网 IP（应答来源或目的 local addr）
    pub ip: String,
    pub port: u16,
    pub name: String,
}

fn parse_reply(packet: &[u8], fallback_ip: &str) -> Option<DiscoveredServer> {
    let text = std::str::from_utf8(packet).ok()?;
    // 两种前缀都要认：服务端按客户端探测的版本回显，故任一版本都可能出现在应答里
    let body = [REPLY_PREFIX, LEGACY_REPLY_PREFIX]
        .iter()
        .find_map(|prefix| text.trim().strip_prefix(prefix))?;
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(DiscoveredServer {
        ip: fallback_ip.to_string(),
        port: v["port"].as_u64()? as u16,
        name: v["name"].as_str().unwrap_or("SKYLAND 服务端").to_string(),
    })
}

/// 探测包 → 对应的应答前缀（版本回显）。
/// 返回 `None` 表示不是本协议的探测包，服务端应静默忽略。
fn reply_prefix_for(probe: &[u8]) -> Option<&'static str> {
    if probe == PROBE_PACKET {
        Some(REPLY_PREFIX)
    } else if probe == LEGACY_PROBE_PACKET {
        Some(LEGACY_REPLY_PREFIX)
    } else {
        None
    }
}

/// 服务端应答线程（阻塞循环 — 调用方自行 spawn 到独立线程）
/// `http_port`: HTTP 服务端口；`server_name`: 显示名（app.toml server_name）
pub fn responder_loop(http_port: u16, server_name: &str) -> std::io::Result<()> {
    let socket = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT))?;
    eprintln!("✓ 局域网发现应答已启动 (UDP {})", DISCOVERY_PORT);
    let mut buf = [0u8; 256];
    loop {
        let (len, src) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // 版本回显：按客户端探测的版本作答，保证对方一定能解析
        let Some(prefix) = reply_prefix_for(&buf[..len]) else { continue; };
        // 客户端以应答包来源地址推导服务端局域网 IP，应答体只需 name+port
        let reply = format!(
            "{}{}", prefix,
            serde_json::json!({ "name": server_name, "port": http_port })
        );
        let _ = socket.send_to(reply.as_bytes(), src);
    }
}

/// 客户端扫描：广播探测 + 收集应答（阻塞 — Tauri 命令/托盘在专用线程调用）
pub fn discover_servers(timeout_ms: u64) -> Vec<DiscoveredServer> {
    let socket = match UdpSocket::bind(("0.0.0.0", 0)) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    socket.set_broadcast(true).ok();
    socket.set_read_timeout(Some(Duration::from_millis(200))).ok();

    let targets = [
        ("255.255.255.255", DISCOVERY_PORT), // 全网广播
        ("127.0.0.1", DISCOVERY_PORT),       // 本机服务端
    ];
    // 两种探测包都发：只发新包会漏掉未升级的旧服务端，只发旧包则旧客户端仍能看到新版
    for probe in [PROBE_PACKET, LEGACY_PROBE_PACKET] {
        for (host, port) in targets {
            let _ = socket.send_to(probe, (host, port));
        }
    }
    // 重发一次提高丢包容错
    std::thread::sleep(Duration::from_millis(200));
    for probe in [PROBE_PACKET, LEGACY_PROBE_PACKET] {
        for (host, port) in targets {
            let _ = socket.send_to(probe, (host, port));
        }
    }

    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms.max(400));
    let mut found: Vec<DiscoveredServer> = Vec::new();
    let mut buf = [0u8; 512];
    while std::time::Instant::now() < deadline {
        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                // 服务端 IP = 应答来源地址（广播/单播应答的路由回程必然可达）
                if let Some(server) = parse_reply(&buf[..len], &src.ip().to_string()) {
                    if !found.iter().any(|f| f.ip == server.ip && f.port == server.port) {
                        found.push(server);
                    }
                }
            }
            Err(_) => {
                // 超时继续轮询直到 deadline
            }
        }
    }
    found.sort_by(|a, b| a.ip.cmp(&b.ip));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_reply() {
        let body = format!("{}{}", REPLY_PREFIX,
            serde_json::json!({ "name": "我的服务端", "port": 3000 }));
        let s = parse_reply(body.as_bytes(), "192.168.1.10").unwrap();
        assert_eq!(s.ip, "192.168.1.10");
        assert_eq!(s.port, 3000);
        assert_eq!(s.name, "我的服务端");
        assert!(parse_reply(b"garbage", "1.2.3.4").is_none());
        assert!(parse_reply(b"MOLINE_V1 not-json", "1.2.3.4").is_none());
    }

    /// 线协议兼容：旧版服务端（MOLINE_V1 前缀）发来的应答，新客户端必须能解析。
    /// 这条断言守护的是"改名前后客户端/服务端交叉升级"的可用性。
    #[test]
    fn test_parse_reply_accepts_legacy_prefix() {
        let body = format!("{}{}", LEGACY_REPLY_PREFIX,
            serde_json::json!({ "name": "旧版服务端", "port": 3001 }));
        let s = parse_reply(body.as_bytes(), "192.168.1.11").unwrap();
        assert_eq!(s.port, 3001);
        assert_eq!(s.name, "旧版服务端");
    }

    /// 版本回显：旧探测包必须拿到旧前缀，否则旧客户端解析不了（等于对旧客户端隐身）。
    #[test]
    fn test_legacy_probe_gets_legacy_reply_prefix() {
        assert_eq!(reply_prefix_for(LEGACY_PROBE_PACKET), Some(LEGACY_REPLY_PREFIX));
    }

    #[test]
    fn test_new_probe_gets_new_reply_prefix() {
        assert_eq!(reply_prefix_for(PROBE_PACKET), Some(REPLY_PREFIX));
    }

    /// 非本协议流量必须被忽略，否则会对局域网任意 UDP 垃圾包作出应答（放大面）。
    #[test]
    fn test_foreign_probe_is_ignored() {
        assert_eq!(reply_prefix_for(b"SOMEONE_ELSE_V1"), None);
        assert_eq!(reply_prefix_for(b""), None);
    }

    /// 端到端互操作（真实 socket）。`responder_loop` 独占固定端口 3311，
    /// 故新旧两个方向必须合并在同一个测试里 —— 拆成两个会并行抢端口而互相失败。
    ///
    /// 覆盖两个方向：
    ///   A. 新客户端 → 新服务端：`discover_servers` 能发现；
    ///   B. 旧客户端 → 新服务端：旧探测包拿到**旧前缀**应答（版本回显），
    ///      否则改名前后交叉升级的旧客户端会静默失联。
    #[test]
    fn test_discovery_end_to_end_both_protocol_versions() {
        std::thread::spawn(|| responder_loop(3999, "测试服务端"));
        std::thread::sleep(std::time::Duration::from_millis(300));

        // A. 新客户端发现新版服务端
        let found = discover_servers(1200);
        let hit = found.iter().find(|s| s.port == 3999);
        assert!(hit.is_some(), "本机服务端应被发现: {:?}", found);
        assert_eq!(hit.unwrap().name, "测试服务端");

        // B. 旧客户端探测 → 断言版本回显为旧前缀
        let probe = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        probe.set_read_timeout(Some(Duration::from_millis(1500))).unwrap();
        probe.send_to(LEGACY_PROBE_PACKET, ("127.0.0.1", DISCOVERY_PORT)).unwrap();
        let mut buf = [0u8; 512];
        let (len, _) = probe.recv_from(&mut buf).expect("旧探测包必须得到应答");
        let text = std::str::from_utf8(&buf[..len]).unwrap();
        assert!(
            text.starts_with(LEGACY_REPLY_PREFIX),
            "旧探测包必须回显旧前缀，实际收到: {:?}",
            text
        );
        // 且旧前缀的应答体新客户端同样要能解析（回显内容格式未变）
        let parsed = parse_reply(&buf[..len], "127.0.0.1").expect("回显应答应可解析");
        assert_eq!(parsed.port, 3999);
    }
}
