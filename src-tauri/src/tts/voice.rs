// src-tauri/src/tts/voice.rs
//
// 캐릭터 → ElevenLabs 보이스 배정.
//
// 두 층으로 결정한다.
//  1) **수동 지정**(프로필 `voiceId`)이 있으면 그것. 목록에서 사라졌으면 자동으로 내려간다.
//  2) **자동 캐스팅**: archetype이 선호하는 라벨(성별·연령)로 후보를 좁힌 뒤,
//     그 안에서 seed 해시로 결정적으로 고른다.
//
// 요구는 여전히 "같은 캐릭터는 항상 같은 목소리"다. 그래서 배정은 seed(없으면
// agentId)의 sha256을 **정렬된** 후보 목록 길이로 나눈 나머지다. 정렬이 중요하다
// — `GET /v2/voices`의 반환 순서는 계약이 아니라서, 정렬 없이는 API가 순서만
// 바꿔도 캐릭터 목소리가 통째로 바뀐다. 필터도 순서를 보존하므로 후보 목록
// 역시 정렬 상태다.
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

/// 라벨 키 — ElevenLabs `labels` 맵에서 캐스팅에 쓰는 두 축.
pub const LABEL_GENDER: &str = "gender";
pub const LABEL_AGE: &str = "age";

/// 배정에 필요한 최소 정보 + 캐스팅용 라벨.
///
/// `labels`는 `voice.labels` 맵을 키 기준 정렬해 담은 것이다(맵을 쓰지 않는
/// 이유: `VoiceRef`가 `PartialEq`·비교 가능해야 하고, 항목이 서너 개뿐이라
/// 선형 탐색이 더 싸다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceRef {
    pub voice_id: String,
    pub name: String,
    pub labels: Vec<(String, String)>,
}

impl VoiceRef {
    /// 라벨 조회(키는 대소문자 무시). 없으면 None.
    pub fn label(&self, key: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn gender(&self) -> Option<&str> {
        self.label(LABEL_GENDER)
    }

    pub fn age(&self) -> Option<&str> {
        self.label(LABEL_AGE)
    }

    /// 설정/프로필 드롭다운에 붙일 짧은 라벨 요약(예: "female · young · american").
    /// 값만 이어 붙인다 — 키까지 붙이면 좁은 select에서 읽히지 않는다.
    pub fn label_summary(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for key in [LABEL_GENDER, LABEL_AGE, "accent", "descriptive"] {
            if let Some(v) = self.label(key) {
                let v = v.trim();
                if !v.is_empty() && !parts.contains(&v) {
                    parts.push(v);
                }
            }
        }
        parts.join(" · ")
    }
}

/// 목록 조회 실패 시 폴백. ElevenLabs 기본 제공(premade) 보이스로, 계정마다
/// 다르지 않고 오래 안정적인 id들이다. (id, 이름, 성별, 연령) — 라벨이 있어야
/// archetype 캐스팅이 폴백 상황에서도 동작한다(없으면 전부 "선호 없음"으로
/// 뭉개져 캐스팅이 무의미해진다).
pub const FALLBACK_VOICES: &[(&str, &str, &str, &str)] = &[
    ("21m00Tcm4TlvDq8ikWAM", "Rachel", "female", "young"),
    ("AZnzlk1XvdvUeBnXmlld", "Domi", "female", "young"),
    ("EXAVITQu4vr4xnSDxMaL", "Bella", "female", "young"),
    ("ErXwobaYiN019PkySvjV", "Antoni", "male", "young"),
    ("MF3mGyEYCl7XYWbV9V6O", "Elli", "female", "young"),
    ("TxGEqnHWrfWFTfGW9XjX", "Josh", "male", "young"),
    ("VR6AewLTigWG4xSOukaQ", "Arnold", "male", "middle_aged"),
    ("pNInz6obpgDQGcFmaJgB", "Adam", "male", "middle_aged"),
    ("yoZ06aMxZJJ28mfxHNu8", "Sam", "male", "young"),
];

pub fn fallback_voices() -> Vec<VoiceRef> {
    let mut v: Vec<VoiceRef> = FALLBACK_VOICES
        .iter()
        .map(|(id, name, gender, age)| VoiceRef {
            voice_id: (*id).to_string(),
            name: (*name).to_string(),
            labels: vec![
                (LABEL_AGE.to_string(), (*age).to_string()),
                (LABEL_GENDER.to_string(), (*gender).to_string()),
            ],
        })
        .collect();
    v.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
    v
}

/// `{"voices":[{voice_id,name,labels:{...}}]}` → voice_id 정렬된 목록. 비었거나
/// 형태가 다르면 None(호출측이 다음 후보/폴백으로 넘어간다). 순수.
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
                labels: parse_labels(e.get("labels")),
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

/// `labels` 맵 → 키 정렬된 (키, 값) 목록. 문자열이 아닌 값과 빈 값은 버리고,
/// 키·값을 소문자로 정규화한다(계정/버전에 따라 `"Female"`이 오기도 한다).
fn parse_labels(value: Option<&serde_json::Value>) -> Vec<(String, String)> {
    let Some(obj) = value.and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = obj
        .iter()
        .filter_map(|(k, v)| {
            let s = v.as_str()?.trim();
            if s.is_empty() {
                return None;
            }
            Some((k.to_lowercase(), s.to_lowercase()))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// archetype이 선호하는 목소리 결. 빈 슬라이스 = 그 축에 선호 없음.
/// 값은 ElevenLabs 라벨 어휘를 따른다(gender: male/female/neutral,
/// age: young/middle_aged/old).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicePreference {
    pub genders: &'static [&'static str],
    pub ages: &'static [&'static str],
}

impl VoicePreference {
    pub const ANY: Self = Self {
        genders: &[],
        ages: &[],
    };

    fn matches(&self, v: &VoiceRef) -> bool {
        self.gender_matches(v) && self.age_matches(v)
    }

    fn gender_matches(&self, v: &VoiceRef) -> bool {
        // 선호가 없거나, 보이스에 라벨이 없으면(정보 부재) 배제하지 않는다 —
        // 라벨 없는 목록에서 후보가 통째로 비는 것을 막는다.
        match v.gender() {
            _ if self.genders.is_empty() => true,
            Some(g) => self.genders.contains(&g),
            None => true,
        }
    }

    fn age_matches(&self, v: &VoiceRef) -> bool {
        match v.age() {
            _ if self.ages.is_empty() => true,
            Some(a) => self.ages.contains(&a),
            None => true,
        }
    }
}

/// archetype → 선호 라벨. 스프라이트 종족(`renderer/office/gen/archetypes.ts`의
/// `ARCHETYPE_IDS` 8종)과 1:1이다. 어울림은 취향이지만 근거를 하나씩 달아 둔다.
///
/// - `human`: 선호 없음. 사람은 아무 목소리나 어울린다 — 여기서 좁히면 전체
///   캐스팅 분산만 줄어든다(대부분의 캐릭터가 human이다).
/// - `elf`: 젊은 여성/중성 — 가늘고 맑은 결.
/// - `orc`: 나이 든 남성 — 굵고 거친 결.
/// - `beastfolk`: 젊은 목소리(성별 무관) — 활달함.
/// - `robot`: 중성 우선, 없으면 남성 / 중년 이상 — 평평하고 낮게.
/// - `android`: 중성/여성, 젊음~중년 — 사람을 흉내 내되 미묘하게 비어 있는 결.
/// - `slime`: 젊은 여성/중성 — 말랑하고 높은 결.
/// - `ghost`: 나이 든 여성/중성 — 희미하고 서늘한 결.
pub const ARCHETYPE_PREFERENCES: &[(&str, VoicePreference)] = &[
    ("human", VoicePreference::ANY),
    (
        "elf",
        VoicePreference {
            genders: &["female", "neutral"],
            ages: &["young"],
        },
    ),
    (
        "orc",
        VoicePreference {
            genders: &["male"],
            ages: &["middle_aged", "old"],
        },
    ),
    (
        "beastfolk",
        VoicePreference {
            genders: &[],
            ages: &["young"],
        },
    ),
    (
        "robot",
        VoicePreference {
            genders: &["neutral", "male"],
            ages: &["middle_aged", "old"],
        },
    ),
    (
        "android",
        VoicePreference {
            genders: &["neutral", "female"],
            ages: &["young", "middle_aged"],
        },
    ),
    (
        "slime",
        VoicePreference {
            genders: &["female", "neutral"],
            ages: &["young"],
        },
    ),
    (
        "ghost",
        VoicePreference {
            genders: &["female", "neutral"],
            ages: &["old"],
        },
    ),
];

/// archetype id → 선호. 미지/부재/"auto"는 선호 없음(`human`과 같다).
/// "auto"는 렌더러가 저장 시 확정하므로 여기까지 오면 확정 전 값이다.
pub fn preference_for(archetype: Option<&str>) -> VoicePreference {
    let Some(a) = archetype.map(|a| a.trim()) else {
        return VoicePreference::ANY;
    };
    ARCHETYPE_PREFERENCES
        .iter()
        .find(|(id, _)| *id == a)
        .map(|(_, p)| *p)
        .unwrap_or(VoicePreference::ANY)
}

/// 선호로 후보를 좁힌다. **비면 완화한다**: (성별+연령) → (성별만) → 전체.
///
/// 완화가 있어야 하는 이유는 계정마다 보유 보이스가 다르기 때문이다 — 늙은
/// 여성 목소리가 하나도 없는 계정에서 ghost가 발화 불가가 되면 안 된다.
/// 입력 순서를 보존하므로 결과도 정렬 상태다(결정적 배정의 전제).
pub fn filter_voices(voices: &[VoiceRef], pref: VoicePreference) -> Vec<VoiceRef> {
    let both: Vec<VoiceRef> = voices.iter().filter(|v| pref.matches(v)).cloned().collect();
    if !both.is_empty() {
        return both;
    }
    let gender_only: Vec<VoiceRef> = voices
        .iter()
        .filter(|v| pref.gender_matches(v))
        .cloned()
        .collect();
    if !gender_only.is_empty() {
        return gender_only;
    }
    voices.to_vec()
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

/// 최종 배정 — 수동 지정 우선, 없으면 archetype 캐스팅 + 해시. 순수·결정적.
///
/// `manual`이 목록에 없으면(계정에서 지운 보이스, 다른 PC에서 가져온 프로필)
/// 조용히 자동 배정으로 내려간다. 발화가 실패하는 것보다 다른 목소리로라도
/// 말하는 편이 낫다(장식 기능의 강등 원칙).
pub fn assign_voice(
    voices: &[VoiceRef],
    manual: Option<&str>,
    archetype: Option<&str>,
    key: &str,
) -> Option<VoiceRef> {
    if let Some(id) = manual.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(hit) = voices.iter().find(|v| v.voice_id == id) {
            return Some(hit.clone());
        }
        eprintln!("tts: 지정 보이스({id})를 목록에서 찾지 못해 자동 배정으로 폴백");
    }
    let candidates = filter_voices(voices, preference_for(archetype));
    pick_voice(&candidates, key)
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
                labels: Vec::new(),
            })
            .collect();
        out.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
        out
    }

    /// (id, gender, age) → 정렬된 VoiceRef 목록. 라벨 축이 필요한 테스트용.
    fn labeled(rows: &[(&str, &str, &str)]) -> Vec<VoiceRef> {
        let mut out: Vec<VoiceRef> = rows
            .iter()
            .map(|(id, g, a)| VoiceRef {
                voice_id: (*id).to_string(),
                name: (*id).to_string(),
                labels: vec![
                    (LABEL_AGE.to_string(), (*a).to_string()),
                    (LABEL_GENDER.to_string(), (*g).to_string()),
                ],
            })
            .collect();
        out.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
        out
    }

    #[test]
    fn parse_sorts_by_voice_id_so_api_order_does_not_matter() {
        let a = parse_voices(
            r#"{"voices":[{"voice_id":"zz","name":"Z"},{"voice_id":"aa","name":"A"}]}"#,
        );
        let b = parse_voices(
            r#"{"voices":[{"voice_id":"aa","name":"A"},{"voice_id":"zz","name":"Z"}]}"#,
        );
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
    fn parse_reads_labels_lowercased_and_sorted() {
        let out = parse_voices(
            r#"{"voices":[{"voice_id":"a","name":"A","labels":{"Gender":"Female","age":"young","use_case":"","n":3}}]}"#,
        )
        .unwrap();
        assert_eq!(out[0].gender(), Some("female"), "값도 키도 소문자로 정규화");
        assert_eq!(out[0].age(), Some("young"));
        // 빈 문자열/비문자열 라벨은 버린다.
        assert_eq!(out[0].labels.len(), 2, "{:?}", out[0].labels);
        assert_eq!(out[0].labels[0].0, "age", "키 정렬");
    }

    #[test]
    fn parse_tolerates_missing_labels() {
        let out = parse_voices(r#"{"voices":[{"voice_id":"a","name":"A"}]}"#).unwrap();
        assert!(out[0].labels.is_empty());
        assert_eq!(out[0].gender(), None);
    }

    #[test]
    fn label_summary_joins_known_axes_without_duplicates() {
        let vr = VoiceRef {
            voice_id: "a".into(),
            name: "A".into(),
            labels: vec![
                ("accent".into(), "american".into()),
                ("age".into(), "young".into()),
                ("descriptive".into(), "young".into()),
                ("gender".into(), "female".into()),
            ],
        };
        assert_eq!(vr.label_summary(), "female · young · american");
        assert_eq!(v(&["x"])[0].label_summary(), "", "라벨 없으면 빈 문자열");
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
    fn fallback_list_is_nonempty_sorted_and_labeled() {
        let f = fallback_voices();
        assert!(f.len() >= 5);
        let mut sorted = f.clone();
        sorted.sort_by(|a, b| a.voice_id.cmp(&b.voice_id));
        assert_eq!(f, sorted);
        // 라벨이 없으면 폴백 상황에서 archetype 캐스팅이 통째로 무의미해진다.
        assert!(f.iter().all(|v| v.gender().is_some() && v.age().is_some()));
        assert!(f.iter().any(|v| v.gender() == Some("male")));
        assert!(f.iter().any(|v| v.gender() == Some("female")));
    }

    // ── archetype 캐스팅 ─────────────────────────────────────────────
    #[test]
    fn every_sprite_archetype_has_a_preference_entry() {
        // renderer/office/gen/archetypes.ts의 ARCHETYPE_IDS와 같은 8종.
        for id in [
            "human",
            "elf",
            "orc",
            "beastfolk",
            "robot",
            "android",
            "slime",
            "ghost",
        ] {
            assert!(
                ARCHETYPE_PREFERENCES.iter().any(|(a, _)| *a == id),
                "{id} 매핑 누락"
            );
        }
    }

    #[test]
    fn unknown_or_auto_archetype_has_no_preference() {
        assert_eq!(preference_for(None), VoicePreference::ANY);
        assert_eq!(preference_for(Some("auto")), VoicePreference::ANY);
        assert_eq!(preference_for(Some("dragon")), VoicePreference::ANY);
        assert_eq!(preference_for(Some("human")), VoicePreference::ANY);
    }

    #[test]
    fn filter_narrows_to_the_preferred_labels() {
        let voices = labeled(&[
            ("f-young", "female", "young"),
            ("f-old", "female", "old"),
            ("m-young", "male", "young"),
            ("m-old", "male", "old"),
        ]);
        let elf = filter_voices(&voices, preference_for(Some("elf")));
        assert_eq!(
            elf.iter().map(|v| v.voice_id.as_str()).collect::<Vec<_>>(),
            vec!["f-young"]
        );
        let orc = filter_voices(&voices, preference_for(Some("orc")));
        assert_eq!(
            orc.iter().map(|v| v.voice_id.as_str()).collect::<Vec<_>>(),
            vec!["m-old"]
        );
    }

    #[test]
    fn filter_relaxes_age_then_everything_when_nothing_matches() {
        // 늙은 여성이 없는 계정: ghost는 성별만 맞춰 완화한다.
        let no_old = labeled(&[("f-young", "female", "young"), ("m-old", "male", "old")]);
        let ghost = filter_voices(&no_old, preference_for(Some("ghost")));
        assert_eq!(
            ghost.iter().map(|v| v.voice_id.as_str()).collect::<Vec<_>>(),
            vec!["f-young"],
            "연령만 완화하고 성별은 지킨다"
        );

        // 남성밖에 없는 계정: 전체로 완화(발화 불가가 되면 안 된다).
        let males = labeled(&[("m1", "male", "young"), ("m2", "male", "old")]);
        assert_eq!(filter_voices(&males, preference_for(Some("slime"))), males);
    }

    #[test]
    fn filter_keeps_unlabeled_voices_as_candidates() {
        // 라벨 없는 목록(권한 없는 키 등)에서 후보가 통째로 비면 안 된다.
        let plain = v(&["a", "b", "c"]);
        assert_eq!(filter_voices(&plain, preference_for(Some("orc"))), plain);
    }

    #[test]
    fn filter_preserves_sort_order() {
        let voices = labeled(&[
            ("c", "female", "young"),
            ("a", "female", "young"),
            ("b", "female", "young"),
        ]);
        let out = filter_voices(&voices, preference_for(Some("elf")));
        let ids: Vec<&str> = out.iter().map(|v| v.voice_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "정렬이 배정 결정성의 전제다");
    }

    // ── 최종 배정 ────────────────────────────────────────────────────
    #[test]
    fn assign_is_deterministic_and_respects_the_archetype() {
        let voices = labeled(&[
            ("f1", "female", "young"),
            ("f2", "female", "young"),
            ("m1", "male", "old"),
            ("m2", "male", "old"),
        ]);
        let elf = assign_voice(&voices, None, Some("elf"), "seed-1").unwrap();
        assert!(elf.voice_id.starts_with('f'), "{elf:?}");
        for _ in 0..10 {
            assert_eq!(
                assign_voice(&voices, None, Some("elf"), "seed-1"),
                Some(elf.clone())
            );
        }
        let orc = assign_voice(&voices, None, Some("orc"), "seed-1").unwrap();
        assert!(orc.voice_id.starts_with('m'), "{orc:?}");
    }

    #[test]
    fn manual_override_wins_over_archetype_casting() {
        let voices = labeled(&[("f1", "female", "young"), ("m1", "male", "old")]);
        let picked = assign_voice(&voices, Some(" m1 "), Some("elf"), "seed-1").unwrap();
        assert_eq!(picked.voice_id, "m1", "공백은 다듬고 지정은 절대 우선");
    }

    #[test]
    fn manual_override_falls_back_when_the_voice_disappeared() {
        let voices = labeled(&[("f1", "female", "young"), ("f2", "female", "young")]);
        let picked = assign_voice(&voices, Some("deleted-id"), Some("elf"), "seed-1").unwrap();
        assert_eq!(
            Some(picked),
            assign_voice(&voices, None, Some("elf"), "seed-1"),
            "사라진 지정은 자동 배정과 같은 결과여야 한다"
        );
    }

    #[test]
    fn blank_manual_override_is_treated_as_auto() {
        let voices = labeled(&[("f1", "female", "young"), ("f2", "female", "young")]);
        assert_eq!(
            assign_voice(&voices, Some("   "), Some("elf"), "s"),
            assign_voice(&voices, None, Some("elf"), "s")
        );
    }

    #[test]
    fn assign_on_empty_list_is_none() {
        assert_eq!(assign_voice(&[], Some("x"), Some("elf"), "s"), None);
    }
}
