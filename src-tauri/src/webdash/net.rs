// src-tauri/src/webdash/net.rs
//
// 웹 상태판이 어느 주소에 귀를 열지 정한다. 소켓을 열지 않는 순수 판정과
// 이 머신의 인터페이스 주소 열거만 둔다(판정은 주소 배열만 넣어 시험한다).
use std::net::{IpAddr, Ipv4Addr};

use serde::{Deserialize, Serialize};

/// 설정에서 고르는 네트워크.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WebDashBind {
    /// Tailscale 주소(100.64.0.0/10)에만.
    #[default]
    Tailnet,
    /// 사설 LAN 주소(10/8·172.16/12·192.168/16)에만.
    Lan,
    /// 둘 다.
    Both,
}

/// Tailscale CGNAT 대역 100.64.0.0/10.
pub fn is_tailnet(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..128).contains(&o[1])
}

/// 사설 LAN 대역. tailnet 대역은 사설이 아니므로 겹치지 않는다.
pub fn is_lan(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// 이 정책에서 리스너를 붙일 주소들. 루프백은 늘 넣는다 — 이 머신 브라우저로
/// 확인할 수 있게 하고, 밖에는 열리지 않는다. IPv4만 쓴다(불러 줄 주소가 짧다).
pub fn bind_addrs(bind: WebDashBind, local: &[IpAddr]) -> Vec<Ipv4Addr> {
    let mut out = vec![Ipv4Addr::LOCALHOST];
    for ip in local {
        let IpAddr::V4(v4) = ip else { continue };
        let wanted = match bind {
            WebDashBind::Tailnet => is_tailnet(*v4),
            WebDashBind::Lan => is_lan(*v4),
            WebDashBind::Both => is_tailnet(*v4) || is_lan(*v4),
        };
        if wanted && !out.contains(v4) {
            out.push(*v4);
        }
    }
    out
}

/// 이 머신 인터페이스의 IP 주소들. 못 읽으면 빈 목록(루프백만 열린다).
pub fn local_ip_addrs() -> Vec<IpAddr> {
    #[cfg(unix)]
    {
        let Ok(ifaces) = nix::ifaddrs::getifaddrs() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for iface in ifaces {
            let Some(addr) = iface.address else { continue };
            if let Some(v4) = addr.as_sockaddr_in() {
                out.push(IpAddr::V4(Ipv4Addr::from(v4.ip())));
            }
        }
        out
    }
    #[cfg(not(unix))]
    {
        // 열거 API 대신 UDP 소켓의 소스 주소로 대표 주소를 얻는다(패킷은 나가지 않는다).
        let mut out = Vec::new();
        for probe in ["100.100.100.100:80", "192.168.0.1:80", "10.0.0.1:80"] {
            if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
                if sock.connect(probe).is_ok() {
                    if let Ok(addr) = sock.local_addr() {
                        if !out.contains(&addr.ip()) {
                            out.push(addr.ip());
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        IpAddr::V4(s.parse().unwrap())
    }

    #[test]
    fn picks_addresses_by_policy_and_always_loopback() {
        let local = [v4("127.0.0.1"), v4("100.101.2.3"), v4("192.168.1.10"), v4("8.8.8.8")];
        let s = |v: Vec<Ipv4Addr>| v.iter().map(|ip| ip.to_string()).collect::<Vec<_>>();
        assert_eq!(s(bind_addrs(WebDashBind::Tailnet, &local)), ["127.0.0.1", "100.101.2.3"]);
        assert_eq!(s(bind_addrs(WebDashBind::Lan, &local)), ["127.0.0.1", "192.168.1.10"]);
        assert_eq!(s(bind_addrs(WebDashBind::Both, &local)), ["127.0.0.1", "100.101.2.3", "192.168.1.10"]);
    }

    #[test]
    fn tailnet_range_is_cgnat_only() {
        assert!(is_tailnet("100.64.0.1".parse().unwrap()));
        assert!(is_tailnet("100.127.255.254".parse().unwrap()));
        assert!(!is_tailnet("100.128.0.1".parse().unwrap()));
        assert!(!is_lan("100.100.1.1".parse().unwrap()));
    }
}
