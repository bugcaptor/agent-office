// src-tauri/src/link/outbox.rs
//
// 클라이언트가 호스트에 보낼 이벤트·보고를 쌓아 두는 상자. 끊긴 동안에도 쌓고, 붙으면 seq 순서로
// 다시 보낸다. 호스트가 `ack`한 것만 지운다. 같은 `key`로 다시 넣으면 아직 안 보낸 이전 것을 바꾼다
// (상태 스냅숏처럼 마지막 값만 의미 있는 것). 앱을 다시 켜도 남도록 `link-outbox.json`에 쓴다.
//
// `epoch`는 상자가 새로 생길 때마다 바뀐다. 호스트는 epoch가 같을 때만 받은 seq를 기억해 중복을 버린다.
use std::collections::VecDeque;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "link-outbox.json";
/// 쌓아 두는 상한. 넘치면 오래된 것부터 버린다(끊긴 채 오래 돌면 옛 상태는 의미가 없다).
pub const MAX_ITEMS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub seq: u64,
    pub kind: String,
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    epoch: String,
    next_seq: u64,
    items: VecDeque<Item>,
}

pub struct Outbox {
    path: Option<PathBuf>,
    epoch: String,
    next_seq: u64,
    items: VecDeque<Item>,
    /// 이번 연결에서 이미 보낸 마지막 seq(다시 붙으면 0으로).
    sent: u64,
    /// 넘쳐서 버린 수(상태 표시용).
    pub dropped: u64,
}

impl Outbox {
    /// `path`가 없으면 메모리에만 둔다(시험).
    pub fn load(path: Option<PathBuf>) -> Self {
        let stored = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Stored>(&b).ok());
        match stored {
            Some(s) => Self { path, epoch: s.epoch, next_seq: s.next_seq.max(1), items: s.items, sent: 0, dropped: 0 },
            None => Self {
                path,
                epoch: uuid::Uuid::new_v4().simple().to_string(),
                next_seq: 1,
                items: VecDeque::new(),
                sent: 0,
                dropped: 0,
            },
        }
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn push(&mut self, kind: &str, data: serde_json::Value, key: Option<&str>) -> u64 {
        if let Some(key) = key {
            // 아직 보내지 않은 같은 key는 새 값으로 바꾼다. 보낸 것은 ack를 기다리니 그대로 둔다.
            let sent = self.sent;
            self.items.retain(|i| i.seq <= sent || i.key.as_deref() != Some(key));
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.items.push_back(Item { seq, kind: kind.to_string(), data, key: key.map(str::to_string) });
        while self.items.len() > MAX_ITEMS {
            self.items.pop_front();
            self.dropped += 1;
        }
        self.save();
        seq
    }

    /// 호스트가 `seq`까지 받았다.
    pub fn ack(&mut self, seq: u64) {
        let before = self.items.len();
        self.items.retain(|i| i.seq > seq);
        if self.items.len() != before {
            self.save();
        }
    }

    /// 새 연결: 호스트가 이미 받았다고 한 것을 지우고, 처음부터 다시 보낸다.
    pub fn reconnected(&mut self, acked: u64) {
        self.sent = 0;
        self.ack(acked);
    }

    /// 아직 이번 연결에서 보내지 않은 것들(순서대로). 꺼낸 것은 보낸 것으로 친다.
    pub fn take_unsent(&mut self) -> Vec<Item> {
        let sent = self.sent;
        let out: Vec<Item> = self.items.iter().filter(|i| i.seq > sent).cloned().collect();
        if let Some(last) = out.last() {
            self.sent = last.seq;
        }
        out
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let stored = Stored { epoch: self.epoch.clone(), next_seq: self.next_seq, items: self.items.clone() };
        if let Ok(bytes) = serde_json::to_vec(&stored) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                crate::httpapi::set_owner_only(&tmp);
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// 페어링을 지우면 상자도 새로(다른 호스트에 옛 보고를 보내지 않는다).
    pub fn reset(&mut self) {
        *self = Self {
            path: self.path.clone(),
            epoch: uuid::Uuid::new_v4().simple().to_string(),
            next_seq: 1,
            items: VecDeque::new(),
            sent: 0,
            dropped: 0,
        };
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resends_unacked_items_after_reconnect_and_survives_restart() {
        let path = std::env::temp_dir().join(format!("outbox-{}.json", uuid::Uuid::new_v4()));
        let mut o = Outbox::load(Some(path.clone()));
        let epoch = o.epoch().to_string();
        o.push("report", json!(1), None);
        o.push("report", json!(2), None);
        assert_eq!(o.take_unsent().iter().map(|i| i.seq).collect::<Vec<_>>(), [1, 2]);
        assert!(o.take_unsent().is_empty(), "보낸 것은 같은 연결에서 다시 보내지 않는다");
        o.push("report", json!(3), None);
        o.ack(1);
        // 앱을 다시 켜도 ack 안 된 2·3이 남는다.
        let mut o = Outbox::load(Some(path.clone()));
        assert_eq!(o.epoch(), epoch);
        o.reconnected(2);
        assert_eq!(o.take_unsent().iter().map(|i| i.seq).collect::<Vec<_>>(), [3]);
        assert_eq!(o.push("report", json!(4), None), 4, "seq는 이어진다");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn same_key_replaces_only_unsent_items_and_overflow_drops_oldest() {
        let mut o = Outbox::load(None);
        o.push("state", json!("a"), Some("s"));
        o.take_unsent();
        o.push("state", json!("b"), Some("s"));
        o.push("state", json!("c"), Some("s"));
        let rest: Vec<_> = o.items.iter().map(|i| i.data.clone()).collect();
        assert_eq!(rest, [json!("a"), json!("c")], "보낸 a는 ack를 기다리고, 안 보낸 b는 c로 바뀐다");
        for n in 0..MAX_ITEMS {
            o.push("x", json!(n), None);
        }
        assert_eq!(o.len(), MAX_ITEMS);
        assert_eq!(o.dropped, 2);
        let epoch = o.epoch().to_string();
        o.reset();
        assert_eq!(o.len(), 0);
        assert_ne!(o.epoch(), epoch);
    }
}
