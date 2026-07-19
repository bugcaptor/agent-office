// src-tauri/src/bot/mod.rs
//
// 캐릭터 봇 모드(이슈 #57). 탭 단위로 켜고 끄며, 켜진 탭은 담당 저장소의 Gitea
// 이슈/댓글을 폴링해 화이트리스트된 작성자의 슬래시 명령을 감지하고, 그 이슈를
// 세션에 프롬프트로 주입한다. 앱은 읽기 전용 폴링만 하고, 댓글·PR 작성은
// 에이전트가 `tea`로 직접 수행한다. 설계 정본은 docs/bot-mode-design.md.
//
// 런타임 구조는 control 서버(control/mod.rs)의 태스크 생명주기를 본떴다: 탭별로
// tokio 폴링 태스크를 띄우고(oneshot으로 graceful stop), 상태를 Arc<Mutex>로
// 공유해 bot_status가 스냅샷을 읽는다.

pub mod command;
pub mod gitea;
pub mod runner;
pub mod state_store;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::async_runtime::JoinHandle;
use tokio::sync::oneshot;

use crate::types::{BotAgentStatus, BotStatus};
use runner::{BotContext, BotParams, PollOutcome};

/// 한 탭(agentId)의 살아있는 폴링 태스크.
struct RunningBot {
    shutdown: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
    status: Arc<Mutex<BotAgentStatus>>,
    /// 중단 신호(리뷰 H2). oneshot은 진행 중 spawn_blocking(poll_once)이 끝나야
    /// 폴링되므로, 그와 별개로 poll_once가 각 주입 직전 확인하는 즉시성 플래그.
    cancel: Arc<AtomicBool>,
}

/// 봇 폴링 태스크들의 소유자. AppState가 Arc로 보유한다.
#[derive(Default)]
pub struct BotRuntime {
    tasks: Mutex<HashMap<String, RunningBot>>,
}

impl BotRuntime {
    /// 이 탭의 봇 모드를 시작한다(멱등 — 이미 켜져 있으면 현재 상태 반환).
    /// 파라미터 해석(저장소 slug 감지·tea 계정 확인)은 폴링 태스크가 비동기로
    /// 수행하므로, 실패는 이후 `status`의 `error`로 드러난다.
    pub fn start(&self, ctx: Arc<BotContext>, agent_id: String) -> BotAgentStatus {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(rb) = tasks.get(&agent_id) {
            return rb.status.lock().unwrap().clone();
        }
        let status = Arc::new(Mutex::new(BotAgentStatus {
            running: true,
            ..Default::default()
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = oneshot::channel();
        let handle = tauri::async_runtime::spawn(bot_loop(
            ctx,
            agent_id.clone(),
            status.clone(),
            rx,
            cancel.clone(),
        ));
        let snapshot = status.lock().unwrap().clone();
        tasks.insert(
            agent_id,
            RunningBot {
                shutdown: Some(tx),
                handle,
                status,
                cancel,
            },
        );
        snapshot
    }

    /// 이 탭의 봇 모드를 중단한다. 폴링 태스크에 stop 신호를 보내고 등록을
    /// 지운다. 없으면 no-op.
    pub fn stop(&self, agent_id: &str) {
        let removed = self.tasks.lock().unwrap().remove(agent_id);
        if let Some(mut rb) = removed {
            // cancel을 먼저 세워, 진행 중인 poll_once가 남은 주입을 건너뛰게 한다.
            rb.cancel.store(true, Ordering::Relaxed);
            if let Some(tx) = rb.shutdown.take() {
                let _ = tx.send(());
            }
            let _detached = rb.handle;
        }
    }

    /// 종료 훅에서 모든 폴링 태스크를 내린다.
    pub fn stop_all(&self) {
        let mut tasks = self.tasks.lock().unwrap();
        for (_, mut rb) in tasks.drain() {
            rb.cancel.store(true, Ordering::Relaxed);
            if let Some(tx) = rb.shutdown.take() {
                let _ = tx.send(());
            }
        }
    }

    /// 봇 모드가 켜진 탭들의 상태 스냅샷.
    pub fn status(&self) -> BotStatus {
        let tasks = self.tasks.lock().unwrap();
        let mut agents = std::collections::BTreeMap::new();
        for (id, rb) in tasks.iter() {
            agents.insert(id.clone(), rb.status.lock().unwrap().clone());
        }
        BotStatus { agents }
    }
}

/// 폴링 태스크 본체: 파라미터를 한 번 해석한 뒤 주기적으로 poll_once를 돈다.
async fn bot_loop(
    ctx: Arc<BotContext>,
    agent_id: String,
    status: Arc<Mutex<BotAgentStatus>>,
    mut shutdown_rx: oneshot::Receiver<()>,
    cancel: Arc<AtomicBool>,
) {
    let params = match resolve_params(ctx.clone(), agent_id.clone()).await {
        Ok(p) => p,
        Err(e) => {
            // 해석 실패(cwd 없음·tea 미로그인 등): 태스크는 여기서 끝나되 상태에
            // 오류를 남겨 GUI가 원인을 보여준다. 사용자가 stop으로 정리한다.
            let mut s = status.lock().unwrap();
            s.running = false;
            s.error = Some(e);
            return;
        }
    };
    {
        let mut s = status.lock().unwrap();
        s.slug = Some(params.slug.clone());
    }
    // 커서 프라임 + 재시작 잔존 잡 제거(과거 소급 트리거·유령 잡 방지).
    {
        let ctx2 = ctx.clone();
        let params2 = params.clone();
        if let Ok(Err(e)) =
            tauri::async_runtime::spawn_blocking(move || runner::prime(&ctx2, &params2)).await
        {
            status.lock().unwrap().error = Some(e);
        }
    }
    let interval = Duration::from_secs(params.poll_interval_sec);
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => break,
            _ = tokio::time::sleep(interval) => {}
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let ctx2 = ctx.clone();
        let params2 = params.clone();
        let cancel2 = cancel.clone();
        let res = tauri::async_runtime::spawn_blocking(move || {
            runner::poll_once(&ctx2, &params2, &cancel2)
        })
        .await;
        let mut s = status.lock().unwrap();
        match res {
            Ok(Ok(PollOutcome { issue })) => {
                s.issue = issue;
                s.error = None;
            }
            Ok(Err(e)) => {
                s.error = Some(e);
            }
            Err(_) => { /* join 취소/패닉: 다음 주기에 재시도 */ }
        }
    }
}

/// 프로필과 tea/git 상태를 blocking으로 읽어 실행 파라미터를 해석한다.
async fn resolve_params(ctx: Arc<BotContext>, agent_id: String) -> Result<Arc<BotParams>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let profile = runner::load_profile(&ctx.store, &agent_id)
            .ok_or_else(|| "프로필을 찾을 수 없습니다".to_string())?;
        let cwd = profile
            .cwd
            .clone()
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| "이 캐릭터에 작업 폴더(cwd)가 설정되어 있지 않습니다".to_string())?;
        let repo_slug = gitea::detect_slug(Path::new(&cwd))?;
        let owner = gitea::current_user()?;
        let bot = profile.bot.clone().unwrap_or_default();
        let slug = command::effective_slug(&profile.name, bot.slug.as_deref());
        if slug.is_empty() {
            return Err(
                "슬래시 slug를 파생할 수 없습니다 — 캐릭터 이름이 비었거나 특수문자뿐입니다. BotConfig의 slug 별칭을 지정하세요"
                    .to_string(),
            );
        }
        let idle_quiet_ms = bot.idle_quiet_ms.unwrap_or(3000);
        let poll_interval_sec = bot.poll_interval_sec.unwrap_or(60).max(30);
        Ok(Arc::new(BotParams {
            agent_id,
            name: profile.name,
            repo_slug,
            owner,
            slug,
            whitelist: bot.whitelist,
            idle_quiet_ms,
            poll_interval_sec,
        }))
    })
    .await
    .map_err(|e| format!("파라미터 해석 태스크 실패: {e}"))?
}
