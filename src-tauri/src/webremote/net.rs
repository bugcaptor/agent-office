// src-tauri/src/webremote/net.rs
//
// "어디에 붙고 누구를 받아 줄 것인가" — 주소 계층. 서버 수명·라우트·인증과
// 무관한 순수 판정·탐지만 모아 둔다(그래서 여기 함수는 전부 단위 테스트가
// 쉽다: 소켓을 열지 않고 주소 배열만 넣으면 된다).
//
// 설계 문서의 "tailscale 인터페이스에만 바인드"는 인터페이스 열거 크레이트를
// 새로 들이지 않고 **원격 주소 허용목록 + 바인드 IP 선택**의 조합으로 등가
// 구현한다 — 자세한 근거는 `bind_policy_allows`/`choose_bind_ip` 주석에 있다.
use std::net::IpAddr;

use crate::persistence::settings_store::WebRemoteBind;

/// tailnet(Tailscale CGNAT 대역 100.64.0.0/10) 주소인가.
pub fn is_tailnet_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (64..128).contains(&o[1])
        }
        // Tailscale IPv6는 fd7a:115c:a1e0::/48.
        IpAddr::V6(v6) => {
            let s = v6.segments();
            s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
        }
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

/// 이 원격 주소를 정책상 받아 줄 것인가.
///
/// 설계 문서의 "tailscale 인터페이스에만 바인드"는 인터페이스 열거 크레이트를
/// 새로 들이지 않고 **원격 주소 허용목록**으로 등가 구현한다 — 포트는 열리되
/// tailnet 밖 클라이언트는 페어링·WS 어느 것도 시작하지 못하므로, "기본
/// 구성에서 평문이 LAN에 흐르지 않는다"는 보안 성질은 그대로다.
pub fn bind_policy_allows(bind: WebRemoteBind, ip: IpAddr) -> bool {
    match bind {
        WebRemoteBind::Tailnet => is_tailnet_addr(ip) || is_loopback(ip),
        WebRemoteBind::All => true,
        WebRemoteBind::Loopback => is_loopback(ip),
    }
}

/// 리스너를 실제로 붙일 주소와 tailnet 탐지 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindChoice {
    pub ip: IpAddr,
    /// 로컬 인터페이스에서 tailnet 주소를 찾았는가. `Tailnet` 정책일 때
    /// false면 루프백 폴백이고, 설정 UI가 "tailscale 미탐지"를 띄운다.
    pub tailnet_found: bool,
}

/// 정책 + 이 머신의 로컬 주소 목록 → 바인드할 주소. **순수 함수**라 실제
/// 인터페이스 없이 단위 테스트한다(주소 목록이 곧 입력이다).
///
/// `Tailnet`(기본)은 tailscale 인터페이스 주소에만 리스너를 연다 — 전
/// 인터페이스에 열고 원격 주소로 거르던 예전 방식보다 노출 표면이 작다
/// (tailnet 밖에서는 포트 자체가 닫혀 보인다). IPv4를 우선하는 것은
/// 사용자에게 불러 줄 주소가 짧아야 하기 때문이다.
pub fn choose_bind_ip(bind: WebRemoteBind, addrs: &[IpAddr]) -> BindChoice {
    let tailnet = addrs
        .iter()
        .copied()
        .find(|ip| ip.is_ipv4() && is_tailnet_addr(*ip))
        .or_else(|| addrs.iter().copied().find(|ip| is_tailnet_addr(*ip)));
    let loopback = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    match bind {
        WebRemoteBind::Loopback => BindChoice {
            ip: loopback,
            tailnet_found: tailnet.is_some(),
        },
        WebRemoteBind::All => BindChoice {
            ip: IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            tailnet_found: tailnet.is_some(),
        },
        // 못 찾으면 서버는 띄우되 루프백에만 연다 — 조용히 전 인터페이스로
        // 넓히면 "tailnet만 허용"이라는 설정이 거짓말이 된다.
        WebRemoteBind::Tailnet => match tailnet {
            Some(ip) => BindChoice {
                ip,
                tailnet_found: true,
            },
            None => BindChoice {
                ip: loopback,
                tailnet_found: false,
            },
        },
    }
}

/// 이 머신에 붙어 있는 로컬 IP 주소들.
///
/// unix는 `getifaddrs(3)` 인터페이스 열거(nix — portable-pty가 이미 끌고 오는
/// 의존이라 트리가 늘지 않는다). 그 외 플랫폼은 열거 API가 없어 UDP 소켓의
/// 소스 주소를 캐내는 고전적 방법으로 대신한다(실제 패킷은 나가지 않는다) —
/// tailnet 대역으로 "연결"하면 라우팅 테이블이 tailscale 인터페이스 주소를
/// 골라 준다.
pub fn local_ip_addrs() -> Vec<IpAddr> {
    #[cfg(unix)]
    {
        match nix::ifaddrs::getifaddrs() {
            Ok(ifaces) => {
                let mut out = Vec::new();
                for iface in ifaces {
                    let Some(addr) = iface.address else { continue };
                    if let Some(v4) = addr.as_sockaddr_in() {
                        out.push(IpAddr::V4(std::net::Ipv4Addr::from(v4.ip())));
                    } else if let Some(v6) = addr.as_sockaddr_in6() {
                        out.push(IpAddr::V6(v6.ip()));
                    }
                }
                return out;
            }
            Err(e) => {
                eprintln!("webremote: getifaddrs 실패({e}) — 소켓 프로브로 대체");
            }
        }
    }
    probe_local_addrs()
}

/// 인터페이스 열거를 못 쓰는 경로의 대안. tailnet 대역과 공용 대역 각각에
/// "연결"해 보고 소스 주소를 모은다(UDP connect는 패킷을 보내지 않는다).
fn probe_local_addrs() -> Vec<IpAddr> {
    let mut out = Vec::new();
    for target in [("100.100.100.100", 80u16), ("8.8.8.8", 80)] {
        let Ok(sock) = std::net::UdpSocket::bind(("0.0.0.0", 0)) else {
            continue;
        };
        if sock.connect(target).is_ok() {
            if let Ok(local) = sock.local_addr() {
                if !out.contains(&local.ip()) {
                    out.push(local.ip());
                }
            }
        }
    }
    out
}

/// 이 머신을 사람이 알아볼 이름(호스트 승인/뷰어 목록에 표시).
pub fn local_host_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "agent-office".into())
}

/// 브라우저에 불러 줄 주소(설정 UI 표시용). 서버가 떠 있으면 **실제 바인드
/// 주소**가 정답이고, 아직 안 떴으면 tailnet 주소를 추정해 미리 보여준다.
pub fn local_addr_hint() -> Option<String> {
    let addrs = local_ip_addrs();
    addrs
        .iter()
        .copied()
        .find(|ip| ip.is_ipv4() && is_tailnet_addr(*ip))
        .or_else(|| addrs.iter().copied().find(|ip| is_tailnet_addr(*ip)))
        .or_else(|| probe_local_addrs().into_iter().next())
        .map(|ip| ip.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn tailnet_range_detection() {
        assert!(is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 107, 46, 116))));
        assert!(is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 127, 255, 254))));
        // 경계 밖
        assert!(!is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 63, 0, 1))));
        assert!(!is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))));
        assert!(!is_tailnet_addr(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5))));
    }

    #[test]
    fn tailnet_policy_binds_the_tailscale_interface_address() {
        let addrs = [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5)),
            IpAddr::V4(Ipv4Addr::new(100, 107, 46, 116)),
        ];
        let choice = choose_bind_ip(WebRemoteBind::Tailnet, &addrs);
        assert_eq!(choice.ip, IpAddr::V4(Ipv4Addr::new(100, 107, 46, 116)));
        assert!(choice.tailnet_found);
    }

    #[test]
    fn tailnet_policy_falls_back_to_loopback_when_tailscale_is_absent() {
        // tailscale이 없는 기계 — 서버는 뜨되 LAN에는 절대 열리지 않는다.
        let addrs = [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5)),
        ];
        let choice = choose_bind_ip(WebRemoteBind::Tailnet, &addrs);
        assert_eq!(choice.ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(!choice.tailnet_found, "설정 UI가 미탐지를 알려야 한다");

        // 주소를 하나도 못 구한 경우도 같은 폴백.
        let empty = choose_bind_ip(WebRemoteBind::Tailnet, &[]);
        assert_eq!(empty.ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(!empty.tailnet_found);
    }

    #[test]
    fn tailnet_choice_prefers_ipv4_but_takes_ipv6_when_thats_all_there_is() {
        let v6: IpAddr = "fd7a:115c:a1e0::1".parse().unwrap();
        let both = [v6, IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))];
        assert_eq!(
            choose_bind_ip(WebRemoteBind::Tailnet, &both).ip,
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
            "불러 줄 주소가 짧아야 한다"
        );
        assert_eq!(choose_bind_ip(WebRemoteBind::Tailnet, &[v6]).ip, v6);
    }

    #[test]
    fn all_and_loopback_policies_ignore_the_tailnet_address() {
        let addrs = [IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))];
        let all = choose_bind_ip(WebRemoteBind::All, &addrs);
        assert_eq!(all.ip, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert!(all.tailnet_found, "탐지 결과 자체는 정책과 무관하게 보고한다");
        let lo = choose_bind_ip(WebRemoteBind::Loopback, &addrs);
        assert_eq!(lo.ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn tailnet_policy_rejects_plain_lan_but_allows_loopback() {
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 5));
        let tail = IpAddr::V4(Ipv4Addr::new(100, 64, 1, 2));
        let local = IpAddr::V4(Ipv4Addr::LOCALHOST);
        assert!(!bind_policy_allows(WebRemoteBind::Tailnet, lan));
        assert!(bind_policy_allows(WebRemoteBind::Tailnet, tail));
        assert!(bind_policy_allows(WebRemoteBind::Tailnet, local));

        assert!(bind_policy_allows(WebRemoteBind::All, lan));
        assert!(!bind_policy_allows(WebRemoteBind::Loopback, lan));
        assert!(!bind_policy_allows(WebRemoteBind::Loopback, tail));
        assert!(bind_policy_allows(WebRemoteBind::Loopback, local));
    }
}
