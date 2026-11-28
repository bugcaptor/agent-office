// src-tauri/src/files/engine.rs
//
// 전송 진행(호스트, 또는 이 머신 안의 전송). 원본 쪽에서 청크를 몇 개 앞서 읽고(`READ_AHEAD`) 받는 쪽에는
// 순서대로 쓴다 — 받는 쪽이 앞에서부터 해시를 이어 가고, 이어 보내기 기준(`progress`)이 언제나 받은 쪽에
// 확실히 쓴 데까지가 되게. 원본·받는 쪽은 이 머신(로컬 파일)이거나 붙은 기기(연동 물음)다.
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use futures_util::StreamExt;
use serde_json::json;

use super::model::{Transfer, TransferState, CHUNK};
use super::FilesState;
use crate::link::files as linkfiles;

/// 원본에서 미리 읽어 둘 청크 수.
const READ_AHEAD: usize = 3;
/// 청크 하나를 읽거나 쓰는 물음의 기다림.
const CHUNK_TIMEOUT: Duration = Duration::from_secs(120);
/// 받는 쪽이 다 받은 것을 확인(해시)·옮기는 물음의 기다림.
const FINISH_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 진행 상태를 붙은 기기에 내려 보내는 간격.
const BROADCAST_EVERY: Duration = Duration::from_millis(700);

#[derive(Clone, Debug, PartialEq)]
pub enum End {
    Local,
    Device(String),
}

pub fn endpoint(files: &FilesState, machine_id: &str) -> Result<End, String> {
    if machine_id == files.machine_id {
        return Ok(End::Local);
    }
    let link = files.link().ok_or("link-offline")?;
    let device = link
        .host
        .status()
        .devices
        .into_iter()
        .find(|d| d.machine_id == machine_id)
        .ok_or("file-target-unknown")?;
    if !device.connected {
        return Err("link-device-offline".into());
    }
    Ok(End::Device(device.id))
}

async fn ask(files: &FilesState, device: &str, kind: &str, data: serde_json::Value, timeout: Duration) -> Result<serde_json::Value, String> {
    let link = files.link().ok_or("link-offline")?;
    link.host.request(device, kind, data, timeout).await
}

/// 붙은 기기(원본·받는 쪽)에 지금 상태를 내려 보낸다. 호스트만 한다.
pub fn broadcast(files: &FilesState, t: &Transfer, _force: bool) {
    let Some(link) = files.link() else { return };
    if !link.host.enabled() {
        return;
    }
    let public = t.public();
    let mut sent = Vec::new();
    for machine in [&t.from.machine_id, &t.to.machine_id] {
        if let Ok(End::Device(d)) = endpoint(files, machine) {
            if !sent.contains(&d) {
                let _ = link.host.send(&d, linkfiles::CMD_UPDATE, json!({ "transfer": public }));
                sent.push(d);
            }
        }
    }
}

async fn read(files: &Arc<FilesState>, src: &End, t: &Transfer, index: usize, offset: u64, len: u64) -> Result<Vec<u8>, String> {
    match src {
        End::Local => {
            let source = t.sources.get(index).cloned().ok_or("file-source-missing")?;
            let entry = t.files.get(index).cloned().ok_or("bad-request")?;
            tokio::task::spawn_blocking(move || super::manifest::read_chunk(std::path::Path::new(&source), &entry, offset, len))
                .await
                .map_err(|e| e.to_string())?
        }
        End::Device(d) => {
            let answer = ask(files, d, linkfiles::REQ_READ, json!({ "id": t.id, "index": index, "offset": offset, "len": len }), CHUNK_TIMEOUT).await?;
            let data = answer.get("data").and_then(|v| v.as_str()).ok_or("bad-reply")?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "bad-reply".to_string())?;
            if bytes.len() as u64 != len.min(t.files[index].size - offset) {
                return Err("bad-reply".into());
            }
            Ok(bytes)
        }
    }
}

async fn write(files: &Arc<FilesState>, dst: &End, id: &str, index: usize, offset: u64, data: Vec<u8>) -> Result<(), String> {
    match dst {
        End::Local => files.write_chunk(id.to_string(), index, offset, data).await,
        End::Device(d) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
            ask(files, d, linkfiles::REQ_CHUNK, json!({ "id": id, "index": index, "offset": offset, "data": encoded }), CHUNK_TIMEOUT)
                .await
                .map(|_| ())
        }
    }
}

/// 받는 쪽에 받던 것을 버리라고 한다(취소).
pub async fn abort_destination(files: &Arc<FilesState>, t: &Transfer) {
    match endpoint(files, &t.to.machine_id) {
        Ok(End::Local) => files.abort_receive(&t.id),
        Ok(End::Device(d)) => {
            let _ = ask(files, &d, linkfiles::REQ_ABORT, json!({ "id": t.id }), Duration::from_secs(10)).await;
        }
        Err(_) => {}
    }
}

/// 전송 하나를 끝까지(또는 실패·취소까지) 진행한다. `resume`이면 기록의 `progress`부터 잇는다.
pub async fn run(files: Arc<FilesState>, id: String, resume: bool) {
    let Some(cancel) = files.start_running(&id) else { return };
    let result = drive(&files, &id, resume, &cancel).await;
    files.stop_running(&id);
    let cancelled = cancel.load(Ordering::SeqCst);
    let t = match result {
        Ok(()) => files.get(&id),
        Err(e) => {
            if cancelled {
                if let Some(t) = files.get(&id) {
                    abort_destination(&files, &t).await;
                }
            }
            files.update(&id, |t| {
                t.state = if cancelled { TransferState::Cancelled } else { TransferState::Failed };
                t.error = (!cancelled).then_some(e);
            })
        }
    };
    if let Some(t) = t {
        broadcast(&files, &t, true);
        files.cleanup_if_final(&t);
    }
}

async fn drive(files: &Arc<FilesState>, id: &str, resume: bool, cancel: &Arc<std::sync::atomic::AtomicBool>) -> Result<(), String> {
    let t = files.get(id).ok_or("file-transfer-unknown")?;
    if t.state.is_final() {
        return Err("file-already-finished".into());
    }
    let src = endpoint(files, &t.from.machine_id)?;
    let dst = endpoint(files, &t.to.machine_id)?;
    let t = files
        .update(id, |t| {
            t.state = TransferState::Sending;
            t.error = None;
        })
        .ok_or("file-transfer-unknown")?;
    broadcast(files, &t, true);

    // 받는 쪽에 알리고 이어 받을 자리를 받는다.
    let resume_at: Vec<u64> = if resume { t.progress.clone() } else { Vec::new() };
    let offsets: Vec<u64> = match &dst {
        End::Local => {
            // 이 머신이 받는 쪽이면 같은 기록이 받는 기록이다.
            files.accept_offer(t.clone(), resume_at).await?
        }
        End::Device(d) => {
            let answer = ask(files, d, linkfiles::REQ_OFFER, json!({ "transfer": t.public(), "resume": resume_at }), Duration::from_secs(60)).await?;
            serde_json::from_value(answer.get("offsets").cloned().unwrap_or_default()).map_err(|_| "bad-reply".to_string())?
        }
    };
    if offsets.len() != t.files.len() {
        return Err("bad-reply".into());
    }
    let t = files
        .update(id, |t| {
            t.progress = offsets.clone();
            t.done_bytes = offsets.iter().sum();
        })
        .ok_or("file-transfer-unknown")?;

    let chunks: Vec<(usize, u64, u64)> = t
        .files
        .iter()
        .enumerate()
        .flat_map(|(i, f)| {
            let start = offsets[i];
            (0..).map(move |k| start + k * CHUNK).take_while(move |off| *off < f.size).map(move |off| (i, off, CHUNK.min(f.size - off)))
        })
        .collect();
    let reader = {
        let (files, src, t) = (files.clone(), src.clone(), t.clone());
        futures_util::stream::iter(chunks)
            .map(move |(i, off, len)| {
                let (files, src, t) = (files.clone(), src.clone(), t.clone());
                async move { read(&files, &src, &t, i, off, len).await.map(|data| (i, off, data)) }
            })
            .buffered(READ_AHEAD)
    };
    futures_util::pin_mut!(reader);
    let mut last_broadcast = Instant::now();
    while let Some(item) = reader.next().await {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let (i, off, data) = item?;
        let len = data.len() as u64;
        write(files, &dst, id, i, off, data).await?;
        let t = files.update(id, |t| {
            if let Some(p) = t.progress.get_mut(i) {
                *p = off + len;
            }
            t.done_bytes = t.progress.iter().sum();
        });
        if let Some(t) = t.filter(|_| last_broadcast.elapsed() >= BROADCAST_EVERY) {
            broadcast(files, &t, false);
            last_broadcast = Instant::now();
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("cancelled".into());
    }

    // 받는 쪽이 확인하고 옮긴다.
    match &dst {
        End::Local => {
            files.finish_receive(id).await?;
        }
        End::Device(d) => {
            let answer = ask(files, d, linkfiles::REQ_FINISH, json!({ "id": id }), FINISH_TIMEOUT).await?;
            let saved: Vec<String> = serde_json::from_value(answer.get("saved").cloned().unwrap_or_default()).unwrap_or_default();
            files.update(id, |t| {
                t.saved = saved;
                t.state = TransferState::Done;
                t.done_bytes = t.total_bytes;
                t.error = None;
            });
        }
    }
    Ok(())
}
