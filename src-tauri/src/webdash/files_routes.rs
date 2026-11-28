// src-tauri/src/webdash/files_routes.rs
//
// 웹 상태판의 파일 보내기(`crate::files`). 접속한 기기(폰·PC)의 파일을 청크로 올려 이 머신이나 붙은 머신·
// 캐릭터에게 보내고, 보낸·받은 기록을 보고, 받은 파일을 내려받는다. 호스트는 붙은 머신의 받은 파일·캐릭터
// 작업 폴더를 훑어 다른 머신으로 보내라고 지시할 수도 있다(`file-browse`·`file-prepare`).
//
// 올린 파일은 앱 데이터 `file-uploads/<id>/<이름>`에 두었다가 보낼 때 한 폴더로 모으고, 전송이 끝나면 지운다.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Json, Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::Router;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use super::routes::{activity, Dash};
use crate::files::model::{safe_name, CHUNK};
use crate::files::{FilesState, Party, SendRequest, UPLOADS_DIR};
use crate::httpapi::{fail, ok};

type Ctx = State<Arc<Dash>>;

/// 올리기 청크 하나의 상한(브라우저는 4MB씩 보낸다).
const UPLOAD_CHUNK_MAX: usize = 8 * 1024 * 1024;
/// 보내지 않고 남은 올린 파일을 버리기까지.
const UPLOAD_TTL: Duration = Duration::from_secs(24 * 3600);

struct Upload {
    path: PathBuf,
    size: u64,
    received: u64,
    at: Instant,
}

fn uploads() -> &'static Mutex<HashMap<String, Upload>> {
    static U: OnceLock<Mutex<HashMap<String, Upload>>> = OnceLock::new();
    U.get_or_init(Default::default)
}

fn files() -> Result<Arc<FilesState>, Json<Value>> {
    crate::link::files::current().ok_or_else(|| fail("files-unavailable"))
}

pub(super) fn routes() -> Router<Arc<Dash>> {
    Router::new()
        .route("/files", get(overview))
        .route("/files/uploads", post(start_upload))
        .route("/files/uploads/:id", put(put_chunk).layer(DefaultBodyLimit::max(UPLOAD_CHUNK_MAX + 1024)))
        .route("/files/send", post(send))
        .route("/files/browse", get(browse))
        .route("/files/t/:id/:op", post(transfer_op))
        .route("/files/t/:id/file/:index", get(download))
}

/// 머신 id → 붙은 기기 id(호스트). 이 머신이면 None.
fn device_of(machine_id: &str) -> Option<String> {
    let files = crate::link::files::current()?;
    if machine_id == files.machine_id {
        return None;
    }
    let link = files.link()?;
    link.host.status().devices.into_iter().find(|d| d.machine_id == machine_id).map(|d| d.id)
}

/// 기록·대상·설정, 호스트면 훑어볼 수 있는 머신 목록.
async fn overview() -> Json<Value> {
    let files = match files() {
        Ok(f) => f,
        Err(e) => return e,
    };
    let status = files.status();
    let mut sources = vec![json!({ "machineId": files.machine_id, "name": status.machine_name, "local": true, "online": true })];
    if let Some(link) = files.link().filter(|l| l.host.enabled()) {
        for d in link.host.status().devices {
            sources.push(json!({ "machineId": d.machine_id, "name": d.name, "deviceId": d.id, "local": false, "online": d.connected }));
        }
    }
    let mut transfers = files.list();
    transfers.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    ok(json!({ "status": status, "targets": files.targets(), "transfers": transfers, "sources": sources }))
}

#[derive(Deserialize)]
struct StartUpload {
    name: String,
    size: u64,
}

async fn start_upload(Json(b): Json<StartUpload>) -> Json<Value> {
    let files = match files() {
        Ok(f) => f,
        Err(e) => return e,
    };
    let Some(data) = files.data_dir() else { return fail("files-unavailable") };
    let name = safe_name(&b.name);
    let id = format!("u-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let dir = data.join(UPLOADS_DIR).join(&id);
    if let Some(free) = crate::files::receive::free_space(&data.join(UPLOADS_DIR)) {
        if free < b.size + 64 * 1024 * 1024 {
            return fail("file-no-space");
        }
    }
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(dir.join(&name), b"").is_err() {
        return fail("file-write-failed");
    }
    let mut all = uploads().lock().unwrap();
    // 오래 남은 것을 치운다.
    all.retain(|_, u| {
        let keep = u.at.elapsed() < UPLOAD_TTL;
        if !keep {
            if let Some(parent) = u.path.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
        }
        keep
    });
    all.insert(id.clone(), Upload { path: dir.join(&name), size: b.size, received: 0, at: Instant::now() });
    ok(json!({ "id": id }))
}

#[derive(Deserialize)]
struct Offset {
    offset: u64,
}

async fn put_chunk(Path(id): Path<String>, Query(q): Query<Offset>, body: Bytes) -> Json<Value> {
    let (path, size, received) = {
        let all = uploads().lock().unwrap();
        let Some(u) = all.get(&id) else { return fail("file-transfer-unknown") };
        (u.path.clone(), u.size, u.received)
    };
    // 같은 청크를 다시 보내도(응답을 놓친 재시도) 받은 데까지만 이어 쓴다.
    if q.offset > received || q.offset + body.len() as u64 > size {
        return fail("bad-request");
    }
    let end = q.offset + body.len() as u64;
    let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(&path)?;
        f.seek(SeekFrom::Start(q.offset))?;
        f.write_all(&body)
    })
    .await;
    if !matches!(written, Ok(Ok(()))) {
        return fail("file-write-failed");
    }
    let mut all = uploads().lock().unwrap();
    let Some(u) = all.get_mut(&id) else { return fail("file-transfer-unknown") };
    u.received = u.received.max(end);
    u.at = Instant::now();
    ok(json!({ "received": u.received }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendBody {
    /// 올린 파일들(이 머신에서 보낸다).
    #[serde(default)]
    uploads: Vec<String>,
    /// 훑어서 고른 경로와 그 머신(호스트가 붙은 머신에 지시).
    #[serde(default)]
    source_machine_id: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    machine_id: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    notify: bool,
}

async fn send(State(ctx): Ctx, Json(b): Json<SendBody>) -> Json<Value> {
    let files = match files() {
        Ok(f) => f,
        Err(e) => return e,
    };
    let to_label = files
        .targets()
        .into_iter()
        .find(|t| t.machine_id == b.machine_id && t.agent_id == b.agent_id)
        .map(|t| t.spec())
        .unwrap_or_default();
    let result = if !b.uploads.is_empty() {
        send_uploads(&files, &b)
    } else {
        send_paths(&files, &b).await
    };
    match result {
        Ok(t) => {
            activity(&ctx, "file-send", None, Some(&format!("{} → {to_label}", t["items"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default())));
            ok(t)
        }
        Err(e) => fail(&e),
    }
}

fn send_uploads(files: &Arc<FilesState>, b: &SendBody) -> Result<Value, String> {
    let data = files.data_dir().ok_or("files-unavailable")?;
    let batch = data.join(UPLOADS_DIR).join(format!("b-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
    std::fs::create_dir_all(&batch).map_err(|_| "file-write-failed".to_string())?;
    let mut paths = Vec::new();
    {
        let mut all = uploads().lock().unwrap();
        for id in &b.uploads {
            let u = all.get(id).ok_or("file-transfer-unknown")?;
            if u.received < u.size {
                return Err(format!("file-incomplete: {}", u.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
            }
        }
        for id in &b.uploads {
            let u = all.remove(id).ok_or("file-transfer-unknown")?;
            let name = u.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| id.clone());
            let mut dest = batch.join(&name);
            let mut n = 2;
            while dest.exists() {
                dest = batch.join(format!("{n}-{name}"));
                n += 1;
            }
            std::fs::rename(&u.path, &dest).map_err(|_| "file-write-failed".to_string())?;
            if let Some(parent) = u.path.parent() {
                let _ = std::fs::remove_dir_all(parent);
            }
            paths.push(dest.to_string_lossy().into_owned());
        }
    }
    let req = SendRequest {
        machine_id: Some(b.machine_id.clone()),
        agent_id: b.agent_id.clone(),
        paths,
        note: b.note.clone(),
        notify: b.notify || b.note.as_deref().is_some_and(|n| !n.trim().is_empty()),
        via: "web".into(),
        cleanup: Some(batch.clone()),
        ..Default::default()
    };
    match files.send(req) {
        Ok(t) => Ok(json!(t)),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&batch);
            Err(e)
        }
    }
}

async fn send_paths(files: &Arc<FilesState>, b: &SendBody) -> Result<Value, String> {
    if b.paths.is_empty() {
        return Err("file-no-paths".into());
    }
    let notify = b.notify || b.note.as_deref().is_some_and(|n| !n.trim().is_empty());
    let source = b.source_machine_id.clone().unwrap_or_else(|| files.machine_id.clone());
    match device_of(&source) {
        // 이 머신 것: 받은 파일·캐릭터 작업 폴더 안만.
        None => {
            if source != files.machine_id {
                return Err("file-target-unknown".into());
            }
            let data = json!({ "paths": b.paths, "to": party(files, b)?, "note": b.note, "notify": notify, "via": "web" });
            crate::link::files::prepare(files, &data).map(|v| v["transfer"].clone())
        }
        Some(device) => {
            let link = files.link().ok_or("link-offline")?;
            let data = json!({ "paths": b.paths, "to": party(files, b)?, "note": b.note, "notify": notify, "via": "web" });
            let answer = link.host.request(&device, crate::link::files::REQ_PREPARE, data, Duration::from_secs(30)).await?;
            Ok(answer["transfer"].clone())
        }
    }
}

fn party(files: &FilesState, b: &SendBody) -> Result<Party, String> {
    files
        .targets()
        .into_iter()
        .find(|t| t.machine_id == b.machine_id && t.agent_id == b.agent_id)
        .map(|t| t.party())
        .ok_or_else(|| "file-target-unknown".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowseQuery {
    #[serde(default)]
    machine_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

async fn browse(Query(q): Query<BrowseQuery>) -> Json<Value> {
    let files = match files() {
        Ok(f) => f,
        Err(e) => return e,
    };
    let machine = q.machine_id.unwrap_or_else(|| files.machine_id.clone());
    match device_of(&machine) {
        None if machine == files.machine_id => {
            let path = q.path.clone();
            match tokio::task::spawn_blocking(move || crate::link::files::browse(&files, path.as_deref())).await {
                Ok(Ok(v)) => ok(v),
                Ok(Err(e)) => fail(&e),
                Err(e) => fail(&e.to_string()),
            }
        }
        None => fail("file-target-unknown"),
        Some(device) => {
            let Some(link) = files.link() else { return fail("link-offline") };
            match link.host.request(&device, crate::link::files::REQ_BROWSE, json!({ "path": q.path }), Duration::from_secs(20)).await {
                Ok(v) => ok(v),
                Err(e) => fail(&e),
            }
        }
    }
}

async fn transfer_op(State(ctx): Ctx, Path((id, op)): Path<(String, String)>) -> Json<Value> {
    let files = match files() {
        Ok(f) => f,
        Err(e) => return e,
    };
    let result = match op.as_str() {
        "cancel" => files.cancel(&id),
        "resume" => files.resume(&id),
        "forget" => files.forget(&id),
        _ => Err("bad-request".into()),
    };
    match result {
        Ok(()) => {
            if op != "forget" {
                activity(&ctx, &format!("file-{op}"), None, files.get(&id).map(|t| t.items.join(", ")).as_deref());
            }
            ok(json!({}))
        }
        Err(e) => fail(&e),
    }
}

/// 받은 파일 하나를 내려받는다. 이 머신이 받은 것은 디스크에서, 붙은 머신이 받은 것은 청크로 물어 흘려 보낸다.
async fn download(Path((id, index)): Path<(String, usize)>) -> Response {
    let Ok(files) = files() else { return StatusCode::NOT_FOUND.into_response() };
    let Some(t) = files.get(&id).filter(|t| t.state == crate::files::TransferState::Done) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(entry) = t.files.get(index).cloned() else { return StatusCode::NOT_FOUND.into_response() };
    let name = entry.path.rsplit('/').next().unwrap_or("file").to_string();
    let stream: futures_util::stream::BoxStream<'static, Result<Bytes, std::io::Error>> = match device_of(&t.to.machine_id) {
        None if t.to.machine_id == files.machine_id => {
            let probe = {
                let (files, id) = (files.clone(), id.clone());
                tokio::task::spawn_blocking(move || crate::link::files::read_saved(&files, &id, index, 0, 0)).await
            };
            if !matches!(probe, Ok(Ok(_))) {
                return StatusCode::NOT_FOUND.into_response();
            }
            chunked(files.clone(), None, id.clone(), index, entry.size)
        }
        None => return StatusCode::NOT_FOUND.into_response(),
        Some(device) => chunked(files.clone(), Some(device), id.clone(), index, entry.size),
    };
    let mut res = Response::new(Body::from_stream(stream));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if let Ok(v) = HeaderValue::from_str(&entry.size.to_string()) {
        h.insert(header::CONTENT_LENGTH, v);
    }
    let encoded = percent_encoding::utf8_percent_encode(&name, percent_encoding::NON_ALPHANUMERIC).to_string();
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{encoded}")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    res
}

/// 받은 파일을 청크로 읽는 흐름(이 머신 디스크 또는 붙은 머신 `file-saved`).
fn chunked(
    files: Arc<FilesState>,
    device: Option<String>,
    id: String,
    index: usize,
    size: u64,
) -> futures_util::stream::BoxStream<'static, Result<Bytes, std::io::Error>> {
    use futures_util::StreamExt;
    let offsets: Vec<u64> = (0..).map(|k| k * CHUNK).take_while(|o| *o < size).collect();
    futures_util::stream::iter(offsets)
        .then(move |offset| {
            let (files, device, id) = (files.clone(), device.clone(), id.clone());
            async move {
                let err = |e: String| std::io::Error::other(e);
                match device {
                    None => {
                        let read = tokio::task::spawn_blocking(move || crate::link::files::read_saved(&files, &id, index, offset, CHUNK))
                            .await
                            .map_err(|e| err(e.to_string()))?;
                        read.map(|(b, _, _)| Bytes::from(b)).map_err(err)
                    }
                    Some(device) => {
                        let link = files.link().ok_or_else(|| err("link-offline".into()))?;
                        let answer = link
                            .host
                            .request(&device, crate::link::files::REQ_SAVED, json!({ "id": id, "index": index, "offset": offset, "len": CHUNK }), Duration::from_secs(120))
                            .await
                            .map_err(err)?;
                        let data = answer.get("data").and_then(Value::as_str).ok_or_else(|| err("bad-reply".into()))?;
                        base64::engine::general_purpose::STANDARD.decode(data).map(Bytes::from).map_err(|_| err("bad-reply".into()))
                    }
                }
            }
        })
        .boxed()
}
