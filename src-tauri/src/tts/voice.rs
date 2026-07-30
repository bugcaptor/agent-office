// src-tauri/src/tts/voice.rs
//
// 캐릭터 → ElevenLabs 보이스 결정적 배정.
//
// 요구: "같은 캐릭터는 항상 같은 목소리". 그래서 배정은 seed(없으면 agentId)의
// sha256 해시를 **정렬된** 보이스 목록 길이로 나눈 나머지로 고른다. 정렬이
// 중요하다 — `GET /v2/voices`의 반환 순서는 계약이 아니라서, 정렬 없이는 API가
// 순서만 바꿔도 캐릭터 목소리가 통째로 바뀐다.
//
// 목록 조회는 앱 수명 동안 1회만 하고 캐시한다(tts::TtsState). 실패하면 잘
// 알려진 프리메이드 voice_id들로 폴백하므로, 목록 권한이 없는 키에서도 동작한다.

use sha2::{Digest, Sha256};

/// v2가 정식 문서 경로. v1은 문서에서 사라졌지만 여전히 응답하는 경우가 있어
/// 폴백 순서로 남긴다(둘 다 `{"voices":[...]}` 형태).
pub const VOICES_URL_V2: &str = "https://api.elevenlabs.io/v2/voices";
pub const VOICES_URL_V1: &str = "https://api.elevenlabs.io/v1/voices";
/// 목록 조회 타임아웃 — 실패해도 폴백이 있으므로 짧게.
pub const TIMEOUT_SECS: u64 = 8;
/// 한 페이지만 받는다(배정에 필요한 건 "안정적인 후보 집합"이지 전량이 아니다).
pub const PAGE_SIZE: u32 = 100;

/// 배정에 필요한 최소 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceRef {
    pub voice_id: String,
    pub name: String,
}

/// 목록 조회 실패 시 폴백. ElevenLabs 기본 제공(premade) 보이스로, 계정마다
/// 다르지 않고 오래 안정적인 id들이다.
pub const FALLBACK_VOICES: &[(&str, &str)] = &[
    ("21m00Tcm4TlvDq8ikWAM", "Rachel"),
    ("AZnzlk1XvdvUeBnXmlld", "Domi"),
    ("EXAVITQu4vr4xnSDxMaL", "Bella"),
    ("ErXwobaYiN019PkySvjV", "Antoni"),
    ("MF3mGyEYCl7XYWbV9V6O", "Elli"),
    ("TxGEqnHWrfWFTfGW9XjX", "Josh"),
    ("VR6AewLTigWG4xSOukaQ", "Arnold"),
    ("pNInz6obpgDQGcFmaJgB", "Adam"),
    ("yoZ06aMxZJJ28mfxHNu8", "Sam"),
];

pub fn fallback_voices() -> Vec<VoiceRef> {
    let mut v: Vec<VoiceRef> = FALLBACK_VOICES
        .iter()
        .map(|(id, name)| VoiceRef {
            voice_id: (*id).to_string(),
            name: (*name).to_string(),
        })
        .collect();
    v.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
    v
}

/// `{"voices":[{voice_id,name,...}]}` → voice_id 정렬된 목록. 비었거나 형태가
/// 다르면 None(호출측이 다음 후보/폴백으로 넘어간다). 순수.
pub fn parse_voices(body: &str) -> Option<Vec<VoiceRef>> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let arr = v.get("voices")?.as_array()?;
    let mut out: Vec<VoiceRef> = arr
        .iter()
        .filter_map(|e| {
            let id = e.get("voice_id")?.as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            Some(VoiceRef {
                voice_id: id.to_string(),
                name: e
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect();
    if out.is_empty() {
        return None;
    }
    out.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
    out.dedup_by(|a, b| a.voice_id == b.voice_id);
    Some(out)
}

/// `key`(seed 또는 agentId)의 sha256 앞 8바이트를 목록 길이로 모듈로.
/// 목록이 비면 None. 순수·결정적.
pub fn pick_voice(voices: &[VoiceRef], key: &str) -> Option<VoiceRef> {
    if voices.is_empty() {
        return None;
    }
    let digest = Sha256::digest(key.as_bytes());
    let mut n: u64 = 0;
    for b in digest.iter().take(8) {
        n = (n << 8) | u64::from(*b);
    }
    let idx = (n % voices.len() as u64) as usize;
    Some(voices[idx].clone())
}

/// 배정 키: seed가 있으면 seed, 없으면 agentId. seed는 프로필에 영속되므로
/// 캐릭터 이름을 바꿔도 목소리가 유지된다.
pub fn voice_key(agent_id: &str, seed: &str) -> String {
    let s = seed.trim();
    if s.is_empty() {
        agent_id.trim().to_string()
    } else {
        s.to_string()
    }
}

/// 보이스 목록 조회 — v2 → v1 → 하드코딩 폴백. 이 함수만 네트워크를 만진다.
/// 절대 실패하지 않는다(항상 최소 폴백을 돌려준다).
pub async fn fetch_voices(api_key: &str) -> Vec<VoiceRef> {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tts: HTTP 클라이언트 생성 실패 — 폴백 보이스 사용 ({e})");
            return fallback_voices();
        }
    };
    for url in [VOICES_URL_V2, VOICES_URL_V1] {
        let req = client
            .get(url)
            .header("xi-api-key", api_key)
            .query(&[("page_size", PAGE_SIZE.to_string())]);
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    if let Some(v) = parse_voices(&body) {
                        return v;
                    }
                }
            }
            Ok(resp) => eprintln!("tts: 보이스 목록 {url} HTTP {}", resp.status().as_u16()),
            Err(e) => eprintln!("tts: 보이스 목록 {url} 실패 ({e})"),
        }
    }
    eprintln!("tts: 보이스 목록 조회 실패 — 프리메이드 폴백 사용");
    fallback_voices()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(ids: &[&str]) -> Vec<VoiceRef> {
        let mut out: Vec<VoiceRef> = ids
            .iter()
            .map(|i| VoiceRef {
                voice_id: (*i).to_string(),
                name: (*i).to_string(),
            })
            .collect();
        out.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
        out
    }

    #[test]
    fn parse_sorts_by_voice_id_so_api_order_does_not_matter() {
        let a = parse_voices(r#"{"voices":[{"voice_id":"zz","name":"Z"},{"voice_id":"aa","name":"A"}]}"#);
        let b = parse_voices(r#"{"voices":[{"voice_id":"aa","name":"A"},{"voice_id":"zz","name":"Z"}]}"#);
        assert_eq!(a, b);
        assert_eq!(a.unwrap()[0].voice_id, "aa");
    }

    #[test]
    fn parse_rejects_empty_or_malformed() {
        assert_eq!(parse_voices(r#"{"voices":[]}"#), None);
        assert_eq!(parse_voices(r#"{"nope":1}"#), None);
        assert_eq!(parse_voices("not json"), None);
        // voice_id 없는 항목은 건너뛴다.
        assert_eq!(parse_voices(r#"{"voices":[{"name":"X"}]}"#), None);
    }

    #[test]
    fn pick_is_deterministic_for_same_key() {
        let voices = v(&["a", "b", "c", "d", "e"]);
        let first = pick_voice(&voices, "seed-42").unwrap();
        for _ in 0..20 {
            assert_eq!(pick_voice(&voices, "seed-42").unwrap(), first);
        }
    }

    #[test]
    fn pick_differs_across_keys_and_stays_in_range() {
        let voices = v(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let picks: Vec<String> = (0..8)
            .map(|i| pick_voice(&voices, &format!("seed-{i}")).unwrap().voice_id)
            .collect();
        assert!(picks.iter().all(|p| voices.iter().any(|v| &v.voice_id == p)));
        // 8개 키가 전부 같은 목소리로 몰리면 해시가 죽은 것이다.
        assert!(picks.iter().collect::<std::collections::HashSet<_>>().len() > 1);
    }

    #[test]
    fn pick_on_empty_list_is_none() {
        assert_eq!(pick_voice(&[], "seed"), None);
    }

    #[test]
    fn voice_key_prefers_seed_then_agent_id() {
        assert_eq!(voice_key("agent-1", " seed-x "), "seed-x");
        assert_eq!(voice_key("agent-1", "   "), "agent-1");
    }

    #[test]
    fn fallback_list_is_nonempty_and_sorted() {
        let f = fallback_voices();
        assert!(f.len() >= 5);
        let mut sorted = f.clone();
        sorted.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
        assert_eq!(f, sorted);
    }
}
