// src-tauri/src/power.rs
//
// 작업 중 시스템 잠자기 방지(이슈 #68). opt-in 설정 `keep_awake_enabled`가
// 켜져 있고, 렌더러가 "일하는 캐릭터가 하나 이상"이라고 알릴 때만 시스템
// 유휴 잠자기를 막는다. **디스플레이 잠자기는 막지 않는다** — 사용자가 자리를
// 비워도 에이전트는 계속 돌아야 하지만 화면은 꺼져도 무방하다는 의도.
//
// 신호는 렌더러가 주도한다(`set_keep_awake` 커맨드). 렌더러가 "지금 일하는
// 캐릭터가 있음"을 rising-edge에 즉시, 이후 주기적으로 재통지하며, 각 통지는
// lease(TTL 180초)를 갱신한다. 렌더러(webview)가 크래시/행으로 release를
// 놓쳐도 lease가 만료되면 `tick()`이 강제 해제해 assertion 누수를 막는다.
//
// 플랫폼 백엔드:
//   macOS   : IOKit `IOPMAssertionCreateWithName`(PreventUserIdleSystemSleep)
//   Windows : `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`
//             — 스레드 친화적 API라 전담 상주 스레드에서만 호출한다.
//   기타     : no-op

use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// 플랫폼별 웨이크락 구현. `acquire`/`release`는 멱등이어야 한다(중복 호출 무해).
trait PowerBackend: Send {
    fn acquire(&mut self);
    fn release(&mut self);
}

struct Inner {
    backend: Box<dyn PowerBackend>,
    /// assertion을 실제로 쥐고 있는지(백엔드 상태의 미러 — 멱등 게이트).
    active: bool,
    /// 이 시각을 지나면 렌더러 통지가 끊긴 것으로 보고 강제 해제한다.
    lease_until: Option<Instant>,
}

/// 웨이크락 소유자. `Arc`로 공유되어 커맨드 핸들러·lease 감시 태스크·종료
/// 훅에서 함께 쓴다. 클라이언트는 렌더러 하나뿐이므로 refcount가 아니라
/// 단순 불리언 래치로 멱등성을 보장한다.
pub struct WakeLock {
    inner: Mutex<Inner>,
}

impl WakeLock {
    pub fn new() -> Self {
        Self::with_backend(default_backend())
    }

    fn with_backend(backend: Box<dyn PowerBackend>) -> Self {
        Self {
            inner: Mutex::new(Inner {
                backend,
                active: false,
                lease_until: None,
            }),
        }
    }

    /// 잠자기 방지를 활성화하고 lease를 갱신한다. 이미 활성이면 lease만
    /// 연장한다(멱등).
    pub fn renew(&self, lease: Duration) {
        let mut g = self.inner.lock();
        if !g.active {
            g.backend.acquire();
            g.active = true;
        }
        g.lease_until = Some(Instant::now() + lease);
    }

    /// 즉시 해제한다(설정 OFF 전환·앱 종료). 이미 비활성이면 no-op.
    pub fn deactivate(&self) {
        let mut g = self.inner.lock();
        if g.active {
            g.backend.release();
            g.active = false;
        }
        g.lease_until = None;
    }

    /// lease 만료 감시 — 주기 태스크가 호출한다. 활성인데 lease가 지났으면
    /// 강제 해제한다(렌더러 크래시/행 방어).
    pub fn tick(&self) {
        let mut g = self.inner.lock();
        if g.active {
            if let Some(deadline) = g.lease_until {
                if Instant::now() >= deadline {
                    g.backend.release();
                    g.active = false;
                    g.lease_until = None;
                }
            }
        }
    }
}

impl Default for WakeLock {
    fn default() -> Self {
        Self::new()
    }
}

fn default_backend() -> Box<dyn PowerBackend> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacBackend::new())
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WinBackend::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Box::new(NoopBackend)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct NoopBackend;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl PowerBackend for NoopBackend {
    fn acquire(&mut self) {}
    fn release(&mut self) {}
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_void};
    use std::ptr;

    type CFStringRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type IOReturn = i32;
    type IOPMAssertionID = u32;
    type IOPMAssertionLevel = u32;

    const KCFSTRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const KIOPMASSERTION_LEVEL_ON: IOPMAssertionLevel = 255;
    const KIORETURN_SUCCESS: IOReturn = 0;
    // 시스템 유휴 잠자기만 막고 디스플레이 잠자기는 허용.
    const ASSERTION_TYPE: &str = "PreventUserIdleSystemSleep";
    const ASSERTION_NAME: &str = "agent-office: agent working";

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            alloc: CFAllocatorRef,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            assertion_level: IOPMAssertionLevel,
            assertion_name: CFStringRef,
            assertion_id: *mut IOPMAssertionID,
        ) -> IOReturn;
        fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
    }

    pub struct MacBackend {
        id: Option<IOPMAssertionID>,
    }

    impl MacBackend {
        pub fn new() -> Self {
            Self { id: None }
        }
    }

    impl super::PowerBackend for MacBackend {
        fn acquire(&mut self) {
            if self.id.is_some() {
                return;
            }
            // CString::new는 내부 NUL이 없는 리터럴이라 절대 실패하지 않는다.
            let (Ok(type_c), Ok(name_c)) =
                (CString::new(ASSERTION_TYPE), CString::new(ASSERTION_NAME))
            else {
                return;
            };
            unsafe {
                let type_s =
                    CFStringCreateWithCString(ptr::null(), type_c.as_ptr(), KCFSTRING_ENCODING_UTF8);
                let name_s =
                    CFStringCreateWithCString(ptr::null(), name_c.as_ptr(), KCFSTRING_ENCODING_UTF8);
                let mut id: IOPMAssertionID = 0;
                let rc = IOPMAssertionCreateWithName(
                    type_s,
                    KIOPMASSERTION_LEVEL_ON,
                    name_s,
                    &mut id,
                );
                if !type_s.is_null() {
                    CFRelease(type_s);
                }
                if !name_s.is_null() {
                    CFRelease(name_s);
                }
                if rc == KIORETURN_SUCCESS {
                    self.id = Some(id);
                }
            }
        }

        fn release(&mut self) {
            if let Some(id) = self.id.take() {
                unsafe {
                    IOPMAssertionRelease(id);
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::sync::mpsc::{channel, Sender};

    type ExecutionState = u32;
    const ES_CONTINUOUS: ExecutionState = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: ExecutionState = 0x0000_0001;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(es_flags: ExecutionState) -> ExecutionState;
    }

    /// `SetThreadExecutionState`는 호출 스레드에만 적용되고 그 스레드가 살아
    /// 있는 동안만 유지된다 — 그래서 acquire/release를 앱 수명 내내 상주하는
    /// 전담 스레드 하나에서만 호출한다.
    pub struct WinBackend {
        tx: Sender<bool>,
    }

    impl WinBackend {
        pub fn new() -> Self {
            let (tx, rx) = channel::<bool>();
            let _ = std::thread::Builder::new()
                .name("wake-lock".into())
                .spawn(move || {
                    while let Ok(active) = rx.recv() {
                        let flags = if active {
                            ES_CONTINUOUS | ES_SYSTEM_REQUIRED
                        } else {
                            ES_CONTINUOUS
                        };
                        unsafe {
                            SetThreadExecutionState(flags);
                        }
                    }
                    // 채널이 닫히면(앱 종료) 잠자기 방지를 푼다.
                    unsafe {
                        SetThreadExecutionState(ES_CONTINUOUS);
                    }
                });
            Self { tx }
        }
    }

    impl super::PowerBackend for WinBackend {
        fn acquire(&mut self) {
            let _ = self.tx.send(true);
        }
        fn release(&mut self) {
            let _ = self.tx.send(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Default)]
    struct Counts {
        acquires: AtomicUsize,
        releases: AtomicUsize,
    }

    struct FakeBackend {
        counts: Arc<Counts>,
    }

    impl PowerBackend for FakeBackend {
        fn acquire(&mut self) {
            self.counts.acquires.fetch_add(1, Ordering::SeqCst);
        }
        fn release(&mut self) {
            self.counts.releases.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn fake() -> (WakeLock, Arc<Counts>) {
        let counts = Arc::new(Counts::default());
        let lock = WakeLock::with_backend(Box::new(FakeBackend {
            counts: counts.clone(),
        }));
        (lock, counts)
    }

    #[test]
    fn renew_is_idempotent_and_acquires_once() {
        let (lock, counts) = fake();
        lock.renew(Duration::from_secs(180));
        lock.renew(Duration::from_secs(180));
        assert_eq!(counts.acquires.load(Ordering::SeqCst), 1);
        assert_eq!(counts.releases.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn deactivate_releases_once_and_is_idempotent() {
        let (lock, counts) = fake();
        lock.renew(Duration::from_secs(180));
        lock.deactivate();
        lock.deactivate();
        assert_eq!(counts.acquires.load(Ordering::SeqCst), 1);
        assert_eq!(counts.releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn deactivate_without_acquire_is_noop() {
        let (lock, counts) = fake();
        lock.deactivate();
        assert_eq!(counts.acquires.load(Ordering::SeqCst), 0);
        assert_eq!(counts.releases.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn tick_force_releases_after_lease_expiry() {
        let (lock, counts) = fake();
        // lease 0 → 다음 tick 시점에는 반드시 만료(now >= now).
        lock.renew(Duration::from_millis(0));
        lock.tick();
        assert_eq!(counts.releases.load(Ordering::SeqCst), 1);
        // 이미 해제됐으니 추가 tick은 no-op.
        lock.tick();
        assert_eq!(counts.releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn tick_keeps_lock_while_lease_valid() {
        let (lock, counts) = fake();
        lock.renew(Duration::from_secs(3600));
        lock.tick();
        assert_eq!(counts.releases.load(Ordering::SeqCst), 0);
    }
}
