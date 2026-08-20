# tailnet 웹 원격 뷰/제어 설계 — 폐기된 웹 호스팅의 부활과 확장

작성: 2026-08-21 (Fable 검토·계획). 상태: **계획 — 구현 착수 전**.
브랜치: `feature/web-remote` (워크트리 `.claude/worktrees/web-remote`).
선행: `docs/archive/web-hosting-design.md`(#7m/#7n — 2026-08-05 폐기, 이 커밋으로
아카이브 복원) · `docs/cli-control-design.md`(#55).

앱을 띄운 채 tailnet에 웹서버를 호스팅해, 브라우저(폰 포함)로 앱의 상당 부분을
보고 제어한다.

## 1. 타당성 결론

**가능하며, 리스크 낮음.** 근거 세 가지:

1. **완성 직전까지 갔던 전례가 로컬에 온전히 남아 있다.** #7m Phase 1(웹
   클라이언트·allowlist RPC·페어링 보안)은 구현 완료 상태로 폐기됐고, #7n
   Phase 2(tailscale serve HTTPS·가독성)는 설계 확정 상태였다. 폐기 사유는
   기술 실패가 아니라 범위 판단("과한 것 같다")이었다. 커밋 6개가
   `archive/peer-web-hosting`(=`7f924c6`) 태그로 보존돼 있다(2026-08-21 태그
   재생성으로 gc 방어).
2. **이식 충돌이 실측으로 작다.** 아카이브 기반(`77c990b`) 이후 main의 변화는
   커밋 16개·src-tauri 2,651줄 삽입이고, 최대 커밋 `58b177c`(피어 인프라
   5,556줄)를 현 main에 시험 체리픽한 결과 충돌은 `src-tauri/src/lib.rs`
   1개 파일(커맨드 등록 보일러플레이트)뿐이었다.
3. **현 코드베이스가 원격화 친화적으로 진화했다.** 프런트 Tauri 접점은
   `tauriApi.ts` 단일 파사드로 수렴(직접 import 7파일), 백엔드 이벤트는
   `AppEvents` trait로 추상화(`state.rs:25-31`) — WS 브로드캐스터를 3번째
   구현체로 끼우면 된다. 커맨드 대부분이 `State<AppState>`만 받아 axum 재노출이
   쉽고(control 서버가 이미 11개 라우트로 실증), PTY 출력도
   `OutputSink`(채널+backlog 추상, `session/output.rs:29-72`)라 교체 지점이
   명확하다.

## 2. 유지하는 과거 결정 (재검증 완료)

아카이브 문서의 결정 A~L 중 아래를 그대로 계승한다. 근거가 여전히 유효함을
현 코드에서 재확인했다.

- **A. 렌더러 번들 재사용 기각 → 경량 웹 클라이언트(`src/web/`)**.
  기각 사유였던 상태 소유권 문제가 그대로다: `store/persist.ts`의 디바운스
  `saveState` 전량 덮어쓰기(last-writer-wins), 백그라운드 라이터 5종의 이중
  실행(턴 로그·라벨 요약 LLM 호출·일기·스냅샷·메모 청소), `tauriApi` 구체
  모듈을 40개 파일이 직접 import(주입 지점 없음). 웹 클라이언트는 로컬 영속이
  없고 서버 push가 유일한 진실이라 이 문제군이 통째로 소멸한다.
- **B. allowlist RPC — 테이블 밖 `cmd`는 무조건 거부.** 확장도 테이블에 한
  줄씩 추가하는 방식만 허용한다(§4).
- **G. 페어링 인증** — PIN 승인 + 쿠키 30일 + 레이트리밋(pair 3회/분,
  실패 10회/10분 → 429).
- **K. `Tailscale-User-Login` 불신** — 직결 클라이언트가 위조 가능. 신원
  헤더는 인증에도 표시에도 쓰지 않는다.
- **H·I·J. tailscale serve 중간안** — 상태 정본은 tailscaled, 앱은 감지·표시 +
  버튼 대행. 전용 HTTPS 포트 47443(변경 가능), `Secure` 쿠키. `funnel` 영구
  금지, `serve reset` 미사용.
- **admin 티어 없음** — 원격 설정 변경은 셀프 락아웃·권한 상승 표면
  (`webHostingEnabled`를 스스로 끄기, control의 `cliEnabled` 차단과 동일 논리).
- **`session.start`는 저장된 프로필로만** — 웹이 cwd·shell·startupCommand를
  실어 보낼 수 없어야 원격 코드 실행 표면이 되지 않는다.

## 3. 이번에 바꾸는 것

| | 과거(#7m) | 이번 | 근거 |
|---|---|---|---|
| 피어(앱↔앱) 뷰어 | `peer/viewer.rs`(753줄) 동결 유지 | **걷어낸다** — `PeerClientKind::Peer`, 캐릭터별 공유 토글, 앱↔앱 페어링 UI 포함 | 사용자가 폐기한 "피어 접속"은 부활 범위 밖. 웹 전용으로 단순화하면 가시성 규칙(공유 목록 vs tap 판정)의 함정도 소멸 |
| 바인드 | LAN/tailnet(전 인터페이스) | **tailnet 우선** — 기본은 tailscale 인터페이스(100.64/10) IP에 바인드, 미발견 시 안내 후 폴백(설정으로 전 인터페이스 허용) | 사용자 요구가 tailnet 한정. 노출 표면 축소 |
| 웹 화면 | 4개(페어링·목록·터미널·KeyBar) | **"상당 부분"으로 확장** — §4의 3단계 | 이번 요청의 핵심 |
| Phase 2(#7n) | 별도 후속 | **M2로 통합** — serve 연동·Secure 쿠키·폰트 스텝퍼·`Resized` 발행자 신설 | 설계 확정분이므로 그대로 싣는다 |

이름도 "피어/웹 호스팅"이 아니라 **웹 원격(web remote)** 으로 정리한다.
모듈은 `peer/` 대신 `src-tauri/src/webremote/`로 개명 이식한다.

## 4. 기능 범위 — 마일스톤 3단계

### M1. 부활 (아카이브 이식 + 현행화)

체리픽 대상: `58b177c`(인프라) → `738526f`(Phase 1) → `6274419`(페어링
다이얼로그) → `2a3e724`·`af4650f`(dist-web 빌드 체인). 이후 viewer 계열 제거,
`webremote/` 개명, 16커밋치 현행화(외부 attach·workdir 워처·설정 필드 등).

웹에서 되는 것(기존 RPC 6개): 캐릭터 목록·상태 / 알림 보기 / 사용량 스냅샷 /
세션 시작(프로필)·종료 / 알림 클리어 + **터미널 라이브 미러·입력·KeyBar**
(tap·링버퍼·스냅샷 복원, 기존 PeerHub 경로).

완료 기준: 폰 브라우저에서 페어링 → 터미널 보고 입력까지. cargo/vitest 전 통과.

### M2. tailnet HTTPS·가독성 (#7n 설계분 그대로)

- `tailscale serve` 감지·등록/해제 대행(`ipc/commands/tailscale.rs` 신설,
  `parse_serve_status` 순수 함수 + 픽스처 테스트), 포트 47443.
- `Secure` 쿠키(`X-Forwarded-Proto` 판정).
- 웹 터미널 폰트 스텝퍼(9~28px, localStorage)·`overflow: auto` 패닝.
- `HostMsg::Resized` 발행자 신설(`SessionManager::resize` → `AppEvents`).

### M3. 기능 확장 — "상당 부분을 보고 제어"

allowlist에 한 줄씩 추가. 전부 기존 Tauri 커맨드의 재노출이라 백엔드 신규
로직이 거의 없다(대상 커맨드는 `State<AppState>`만 받는 것 우선).

| 묶음 | cmd(안) | 티어 | 원 커맨드 |
|---|---|---|---|
| 작업폴더(읽기) | `workdir.status` `workdir.branch` `workdir.diffFile` `workdir.log` `workdir.commitFiles` `workdir.diffCommit` | viewer | `workdir_*` 12개 중 읽기 6개 |
| 세션 로그 | `sessionlog.list` `sessionlog.read` | viewer | `list_session_logs` 계열 |
| 활동/통계 | `activity.recent` `events.range` | viewer | `load_session_events` |
| 봇 모드 | `bot.status` / `bot.start` `bot.stop` | viewer / operator | `bot_*` 3개 |
| 알림 심화 | `notifications.stream`(WS push) | viewer | `notification-new` 이벤트 중계 |
| 메모(선택) | `memo.load` / `memo.save` | viewer / operator | `load_memo`/`save_memo` |

웹 화면 추가: 캐릭터 상세(브랜치·변경 파일·diff 뷰어 — 렌더러의 의존성 없는
unified diff 렌더 이식) · 봇 제어 버튼 · 알림 티커.

**비범위(영구 또는 재방문 조건부)**: 설정 변경 · 프로필/스프라이트/캐릭터
생성·삭제 · 임의 cwd/명령 스폰 · 오피스 씬·마스코트 · TTS · 파일 쓰기
일반(markdown_write 등) · funnel · 네이티브 다이얼로그/외부 앱 계열
(`open_in_vscode`·`difftool` 등 호스트에서만 유의미한 ~12개) · 웹발 PTY
리사이즈(재방문 조건: M2 후에도 폰 200열 불만 잔존 시).

## 5. 리스크 (기존 분석 계승)

1. **모바일 IME·소프트 키보드 × xterm** — 최대 리스크 유지. KeyBar 별도
   입력칸(조합 완료 후 통째 전송) 전략, iOS Safari 실기기 검증이 게이트.
2. **serve 경유 장수명 WS 끊김**(tailscale #18827) — 20초 ping + 백오프 재접속 +
   offset 복원으로 방어. 토큰은 쿼리스트링 금지(#18651).
3. **이식 부패** — 16커밋치 API 드리프트(관찰자/어댑터 리팩터, 설정 필드 추가,
   외부 attach와 tap의 상호작용). lib.rs 충돌은 작지만 컴파일 후 의미 검증 필요.
4. **다중 클라이언트 동시 입력** — 웹 2탭+네이티브가 같은 PTY에 쓰는 상황은
   tmux와 동일한 "마지막 입력 승"으로 수용, 문서화만.
5. **rust-embed release 빌드** — `dist-web` 미존재 시 컴파일 실패. 빌드 체인
   커밋 2건이 이미 해결분이므로 함께 이식.

## 6. 검증 계획

- cargo: 페어링 레이트리밋 · allowlist 디스패치(미등재 cmd 거부) · tap 게이트 ·
  `parse_serve_status` 픽스처 · `cookie_value` Secure · resize 발행.
- vitest: 웹 클라이언트 protocol 인코딩 · termFont · KeyBar 입력 조립.
- 실기기 눈검증: 폰 페어링→터미널 왕복, serve HTTPS, 웹 attach 중 네이티브
  리사이즈 추종, 토글 즉시 차단(매 요청 `web_remote_enabled` 확인).
