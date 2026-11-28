// src-tauri/src/control/files.rs
//
// `ctl file …`: 머신 간 파일 보내기(`crate::files`). 앱의 보내기와 같은 함수를 부른다.
use axum::extract::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::httpapi::{fail, ok};

#[derive(Deserialize)]
pub(super) struct IdBody {
    id: String,
}

fn files() -> Option<std::sync::Arc<crate::files::FilesState>> {
    crate::link::files::current()
}

pub(super) async fn targets() -> Json<Value> {
    match files() {
        Some(f) => ok(f.targets()),
        None => fail("files-unavailable"),
    }
}

pub(super) async fn list() -> Json<Value> {
    match files() {
        Some(f) => ok(f.list()),
        None => fail("files-unavailable"),
    }
}

pub(super) async fn status(Json(b): Json<IdBody>) -> Json<Value> {
    match files().and_then(|f| f.get(&b.id)) {
        Some(t) => ok(t.public()),
        None => fail("file-transfer-unknown"),
    }
}

pub(super) async fn send(Json(mut req): Json<crate::files::SendRequest>) -> Json<Value> {
    let Some(f) = files() else { return fail("files-unavailable") };
    req.via = "ctl".into();
    req.id = None;
    req.from_agent = None;
    match f.send(req) {
        Ok(t) => ok(t),
        Err(e) => fail(&e),
    }
}

pub(super) async fn resume(Json(b): Json<IdBody>) -> Json<Value> {
    let Some(f) = files() else { return fail("files-unavailable") };
    match f.resume(&b.id) {
        Ok(()) => ok(json!({})),
        Err(e) => fail(&e),
    }
}

pub(super) async fn cancel(Json(b): Json<IdBody>) -> Json<Value> {
    let Some(f) = files() else { return fail("files-unavailable") };
    match f.cancel(&b.id) {
        Ok(()) => ok(json!({})),
        Err(e) => fail(&e),
    }
}
