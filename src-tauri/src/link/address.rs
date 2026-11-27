// src-tauri/src/link/address.rs
//
// 클라이언트가 친 호스트 주소를 읽는다. 자동 발견은 없다. 호스트 설정 화면이 보여 주는 웹 상태판 주소
// (`https://<이름>.ts.net:7788/#t=…`, `https://192.168.0.5:7788/`)를 그대로 붙여도 되고, `192.168.0.5`처럼
// 주소만 써도 된다(포트 기본 7788, 스킴 기본 https). 토큰·경로는 버린다.
//
// 검증 방식은 주소 모양으로 정한다.
// - https + IP: 호스트 앱의 LAN 자체 서명 CA. 페어링 때 본 CA를 고정한다.
// - https + 이름: 공인 인증서(tailnet의 `tailscale cert`).
// - http: 루프백과 tailnet IP만(tailnet은 WireGuard로 이미 암호화된다). 그 밖은 거절한다.
use std::net::Ipv4Addr;

use crate::webdash::net;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAddress {
    pub secure: bool,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustMode {
    /// LAN 자체 서명: 페어링 때 고정한 CA로 검증.
    PinnedCa,
    /// 공인 인증서.
    Public,
    /// 평문(루프백·tailnet IP).
    Plain,
}

impl HostAddress {
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut s = input.trim();
        if let Some(i) = s.find('#') {
            s = &s[..i];
        }
        let (secure, rest) = if let Some(r) = strip_prefix_ci(s, "https://") {
            (true, r)
        } else if let Some(r) = strip_prefix_ci(s, "http://") {
            (false, r)
        } else if s.contains("://") {
            return Err("link-address-invalid".into());
        } else {
            (true, s)
        };
        let authority = rest.split(['/', '?']).next().unwrap_or("");
        if authority.is_empty() || authority.contains('@') || authority.starts_with('[') {
            return Err("link-address-invalid".into());
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "link-address-invalid".to_string())?),
            None => (authority, crate::webdash::DEFAULT_PORT),
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let valid_name = !host.is_empty()
            && host.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        if !valid_name || port == 0 {
            return Err("link-address-invalid".into());
        }
        let addr = Self { secure, host, port };
        if !secure {
            match addr.ip() {
                Some(ip) if ip.is_loopback() || net::is_tailnet(ip) => {}
                _ => return Err("link-address-insecure".into()),
            }
        }
        Ok(addr)
    }

    pub fn ip(&self) -> Option<Ipv4Addr> {
        self.host.parse().ok()
    }

    pub fn trust(&self) -> TrustMode {
        match (self.secure, self.ip()) {
            (false, _) => TrustMode::Plain,
            (true, Some(_)) => TrustMode::PinnedCa,
            (true, None) => TrustMode::Public,
        }
    }

    /// 보기 좋게 다시 쓴 주소(설정에 저장하는 모양).
    pub fn display(&self) -> String {
        format!("{}://{}:{}", if self.secure { "https" } else { "http" }, self.host, self.port)
    }

    pub fn http_url(&self, path: &str) -> String {
        format!("{}{path}", self.display())
    }

    pub fn ws_url(&self, path: &str) -> String {
        format!("{}://{}:{}{path}", if self.secure { "wss" } else { "ws" }, self.host, self.port)
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)).then(|| &s[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pasted_dashboard_urls_and_bare_hosts() {
        let a = HostAddress::parse("https://zm.tail1.ts.net:7788/#t=abc").unwrap();
        assert_eq!(a.display(), "https://zm.tail1.ts.net:7788");
        assert_eq!(a.trust(), TrustMode::Public);
        let a = HostAddress::parse(" 192.168.0.5 ").unwrap();
        assert_eq!(a.display(), "https://192.168.0.5:7788");
        assert_eq!(a.trust(), TrustMode::PinnedCa);
        assert_eq!(a.ws_url("/link/ws"), "wss://192.168.0.5:7788/link/ws");
        let a = HostAddress::parse("HTTP://100.101.2.3:9000/x?y").unwrap();
        assert_eq!((a.trust(), a.port), (TrustMode::Plain, 9000));
    }

    #[test]
    fn refuses_plain_http_off_tailnet_and_junk() {
        assert_eq!(HostAddress::parse("http://192.168.0.5").unwrap_err(), "link-address-insecure");
        assert_eq!(HostAddress::parse("http://example.com").unwrap_err(), "link-address-insecure");
        assert!(HostAddress::parse("http://127.0.0.1:7788").is_ok());
        for bad in ["", "ftp://x", "https://", "x:0", "x:99999", "a b", "user@host", "[::1]:80", "a..b"] {
            assert_eq!(HostAddress::parse(bad).unwrap_err(), "link-address-invalid", "{bad}");
        }
    }
}
