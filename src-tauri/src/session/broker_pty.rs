// src-tauri/src/session/broker_pty.rs
//
// v2 상시 브로커 모드(unix 전용)의 `PtyFactory` 구현. v1(PortablePtyFactory)은
// 앱 프로세스가 PTY master를 직접 소유했지만, 여기서는 데몬(sessiond)이
// 스폰부터 PTY와 자식을 소유하고 앱은 연결만 붙였다 뗀다:
//
//   - spawn(): control 연결로 Spawn RPC -> 별도 연결로 DataAttach(raw 스트림)를
//     열어 reader/writer로 쓰고, 또 다른 연결로 Wait을 담당하게 한다. resize/
//     kill은 control 연결의 RPC로 위임한다. `SpawnedPty` 계약(reader/writer/
//     control/waiter)이 그대로 보존되므로 SessionManager는 사실상 무변경.
//   - 데몬에 닿지 못하면(구버전 데몬 등) `fallback`(보통 PortablePtyFactory)로
//     내려가 프로세스 내 직접 스폰한다 -- v1 경로를 그대로 보존하는 안전장치.
//
// 재접속(adopt)과 스냅샷 업로드/detach는 `SessionManager`가 이 모듈의
// `assemble_broker_adopted`/`connect`를 호출해 수행한다(docs/session-broker-v2-design.md).
#![cfg(unix)]

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::session::pty_factory::{
    ExitOutcome, PtyControl, PtyFactory, PtySpawnOptions, PtyWaiter, SpawnedPty,
};
use crate::sessiond::client::{self, AttachedMeta, Client, SpawnBrokerRequest};

pub struct BrokerPtyFactory {
    socket_path: PathBuf,
    log_path: PathBuf,
    exe_path: PathBuf,
    fallback: Arc<dyn PtyFactory>,
}

impl BrokerPtyFactory {
    /// `fallback`은 브로커에 닿지 못했을 때 쓸 프로세스 내 팩토리(보통
    /// `PortablePtyFactory`). 소켓/로그 경로는 v1과 동일한 규칙으로 유도한다.
    pub fn new(app_data_dir: &Path, fallback: Arc<dyn PtyFactory>) -> Self {
        Self {
            socket_path: client::default_socket_path(app_data_dir),
            log_path: client::default_log_path(app_data_dir),
            exe_path: std::env::current_exe().unwrap_or_default(),
            fallback,
        }
    }

    fn try_broker_spawn(&self, o: &PtySpawnOptions) -> io::Result<SpawnedPty> {
        // control 연결(없으면 데몬 스폰) -- 이후 resize/kill이 이걸 재사용한다.
        let control = client::connect_or_spawn(&self.socket_path, &self.exe_path, &self.log_path)?;
        // 소켓에 이미 떠 있던 구데몬(proto 1)과 협상됐다면 브로커 모드가 불가하다
        // -- 스폰을 시도하기 전에 폴백으로 내려간다(구데몬의 v1 세션은 건드리지
        // 않고, 이 세션만 in-process로 스폰).
        if control.proto() < 2 {
            return Err(io::Error::other(format!(
                "sessiond negotiated proto {} (broker mode needs >= 2)",
                control.proto()
            )));
        }
        control.spawn_broker(SpawnBrokerRequest {
            agent_id: o.agent_id.clone(),
            session_id: o.session_id.clone(),
            shell: o.shell.clone(),
            args: o.args.clone(),
            // 데몬은 앱 재시작을 넘어 존속하므로, 데몬 프로세스가 스폰될 때의
            // (낡을 수 있는) 환경을 그대로 상속시키면 사용자가 셸 rc에서 PATH
            // 등을 바꾼 뒤에도 새 터미널이 옛 환경을 받는다(v1은 앱이 직접 스폰해
            // 늘 실시간 환경). 그래서 앱의 현재 프로세스 환경을 베이스로 실어
            // 보내고, 세션 전용 env(o.env)를 뒤에 둬 마지막에 우선하게 한다
            // (PortablePtyFactory가 상속 위에 o.env를 덮어쓰는 것과 동일 의미).
            env: current_env_with_overrides(&o.env),
            rows: o.rows,
            cols: o.cols,
            cwd: o.cwd.clone(),
            cleanup_paths: o.cleanup_paths.clone(),
        })?;
        // Spawn RPC 성공 후 data/wait 연결 조립이 실패하면(소켓 경합 등) 데몬에
        // 자식만 남는 고아가 된다 -- best-effort Kill로 롤백한 뒤 에러를 돌려
        // 폴백(in-process 스폰)을 타게 한다.
        match open_broker_io(&self.socket_path, &o.agent_id) {
            Ok(io_bundle) => Ok(build_broker_spawned(control, &o.agent_id, io_bundle)),
            Err(e) => {
                let _ = control.kill(&o.agent_id);
                Err(e)
            }
        }
    }
}

/// 앱의 현재 프로세스 환경을 베이스로, 세션 전용 `overrides`를 뒤에 이어 붙인
/// (key, value) 목록. 데몬은 이걸 순서대로 `cmd.env`에 넣으므로 뒤(overrides)가
/// 최종 우선한다. UTF-8이 아닌 env 항목은 프로토콜(JSON)이 실을 수 없어 건너뛴다
/// (대상 플랫폼에서 사실상 없음).
fn current_env_with_overrides(overrides: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| match (k.into_string(), v.into_string()) {
            (Ok(k), Ok(v)) => Some((k, v)),
            _ => None,
        })
        .collect();
    env.extend(overrides.iter().cloned());
    env
}

impl PtyFactory for BrokerPtyFactory {
    fn spawn(&self, o: PtySpawnOptions) -> io::Result<SpawnedPty> {
        match self.try_broker_spawn(&o) {
            Ok(spawned) => Ok(spawned),
            Err(e) => {
                // 폴백은 그 세션을 프로세스 내에서 소유하므로 브로커 존속 이점을
                // 잃지만(구버전 데몬 등 예외 경로), 세션 자체는 정상 동작한다.
                eprintln!("agent-office: broker spawn fell back to in-process PTY: {e}");
                self.fallback.spawn(o)
            }
        }
    }
}

/// resize/kill을 control 연결의 RPC로 위임하는 컨트롤. 여러 스레드가 공유하므로
/// 연결은 Mutex로 감싼다(요청-응답이 원자적으로 오가게).
struct BrokerControl {
    client: Arc<Mutex<Client>>,
    agent_id: String,
}

impl PtyControl for BrokerControl {
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.client.lock().resize(&self.agent_id, rows, cols)
    }
    fn kill(&self) -> io::Result<()> {
        // Kill RPC: 데몬이 자식을 SIGKILL하고 테이블에서 제거한다("모두 종료"/
        // dispose 경로). 이미 종료돼 사라진 세션이면 Error가 오지만 무해.
        let _ = self.client.lock().kill(&self.agent_id);
        Ok(())
    }
}

/// Wait RPC로 자식 종료를 기다리는 대기자(전용 연결). 데몬이 자식의 부모라
/// waitpid가 가능하므로, v1 EofWaiter와 달리 실제 exit code를 돌려준다.
struct BrokerWaiter {
    client: Client,
    agent_id: String,
}

impl PtyWaiter for BrokerWaiter {
    fn wait(self: Box<Self>) -> ExitOutcome {
        match self.client.wait(&self.agent_id) {
            Ok((exit_code, signal)) => ExitOutcome { exit_code, signal },
            // 연결이 먼저 끊긴 경우(앱 종료 중 등)엔 종료 정보를 알 수 없다.
            Err(_) => ExitOutcome { exit_code: None, signal: None },
        }
    }
}

/// data reader를 감싸 누적 수신 바이트를 센다(§P1). `into_data_stream`이 준
/// `stream_offset`으로 카운터를 초기화하고, 읽은 만큼 더한다 -- 스냅샷 업로드가
/// 이 값을 "앱이 실제 여기까지 받았다"는 offset으로 동봉한다.
struct CountingReader {
    inner: UnixStream,
    counter: Arc<AtomicU64>,
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.counter.fetch_add(n as u64, Ordering::SeqCst);
        Ok(n)
    }
}

/// data/wait 연결 번들. 조립 실패 롤백(P2-a)을 위해 control 연결 소비와
/// 분리해 둔다 -- 이게 실패하면 호출자가 아직 control을 쥐고 있어 Kill로
/// 되돌릴 수 있다.
struct BrokerIoBundle {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    wait_client: Client,
    /// data reader의 누적 수신 카운터(스냅샷 offset 동봉용).
    stream_offset: Arc<AtomicU64>,
}

/// data(DataAttach 후 raw 스트림) + wait 연결을 연다. control 연결은 건드리지
/// 않으므로, 이 함수가 실패해도 호출자는 control로 롤백 Kill을 보낼 수 있다.
fn open_broker_io(socket_path: &Path, agent_id: &str) -> io::Result<BrokerIoBundle> {
    // data 연결: reader는 백로그+라이브 출력을, writer는 그대로 PTY master
    // 입력을 담당한다(같은 소켓의 try_clone). DataAttachOk의 stream_offset으로
    // 수신 카운터를 초기화한다(백로그 첫 바이트의 절대 오프셋).
    let (data_stream, stream_offset) = Client::connect(socket_path)?.into_data_stream(agent_id)?;
    let counter = Arc::new(AtomicU64::new(stream_offset));
    let reader: Box<dyn Read + Send> = Box::new(CountingReader {
        inner: data_stream.try_clone()?,
        counter: counter.clone(),
    });
    let writer: Box<dyn Write + Send> = Box::new(data_stream);
    // wait 연결: 종료까지 블로킹하는 전용 연결.
    let wait_client = Client::connect(socket_path)?;
    Ok(BrokerIoBundle { reader, writer, wait_client, stream_offset: counter })
}

/// control 연결 + IO 번들로 `SpawnedPty`를 조립한다. `broker_owned: true` --
/// 이 세션은 데몬이 소유하므로 handoff/adopt가 브로커 경로를 탄다.
fn build_broker_spawned(control: Client, agent_id: &str, io: BrokerIoBundle) -> SpawnedPty {
    let control = Arc::new(Mutex::new(control));
    let pty_control: Arc<dyn PtyControl> = Arc::new(BrokerControl {
        client: control,
        agent_id: agent_id.to_string(),
    });
    let waiter: Box<dyn PtyWaiter> =
        Box::new(BrokerWaiter { client: io.wait_client, agent_id: agent_id.to_string() });

    SpawnedPty {
        reader: io.reader,
        writer: io.writer,
        control: pty_control,
        waiter,
        // 브로커 세션은 fd 핸드오프가 필요 없다(소유권이 이미 데몬에 있다).
        reader_interrupt: None,
        handoff: None,
        broker_owned: true,
        broker_stream_offset: Some(io.stream_offset),
    }
}

/// 재접속(adopt): 이미 데몬 테이블에 있는 브로커 세션에 Attach(메타/스냅샷
/// 회수) + DataAttach(백로그 리플레이 스트림) + Wait 연결을 붙여 `SpawnedPty`와
/// 메타를 조립한다. 조립 실패 시엔 Kill하지 않는다 -- 이 세션은 우리가 만든 게
/// 아니라 이미 데몬에 있던 것이므로 그대로 두고 다음 부팅에서 다시 시도한다.
pub fn assemble_broker_adopted(
    app_data_dir: &Path,
    agent_id: &str,
) -> io::Result<(SpawnedPty, AttachedMeta)> {
    let socket_path = client::default_socket_path(app_data_dir);
    let control = Client::connect(&socket_path)?;
    let meta = control.attach(agent_id)?;
    let io = open_broker_io(&socket_path, agent_id)?;
    Ok((build_broker_spawned(control, agent_id, io), meta))
}

/// 브로커 데몬에 연결한다(없으면 에러 -- 스폰하지 않는다). 스냅샷 업로드/
/// List 등 "이미 떠 있는 데몬에만 의미 있는" 작업에 쓴다.
pub fn connect(app_data_dir: &Path) -> io::Result<Client> {
    Client::connect(&client::default_socket_path(app_data_dir))
}

/// 브로커 데몬 소켓이 존재하는지(=데몬이 떠 있을 가능성이 있는지).
pub fn socket_exists(app_data_dir: &Path) -> bool {
    client::default_socket_path(app_data_dir).exists()
}
