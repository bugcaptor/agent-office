# VSCode 연결형 캐릭터 로그 뷰어 확장 계획

agent-office **데스크톱 앱을 띄운 채로**, VSCode 확장이 앱에 연결해 캐릭터(에이전트)들의
세션 로그를 에디터에서 편하게 스크롤·검색·추적하게 하는 계획의 정본이다.

- 상태: V1 구현 완료 (2026-08-18, `vscode-ext/` — 눈검증 대기, V2·V3 미착수)
- 이슈: kbm #2ce (너른바다/프로젝트관리/AgentOffice)
- 목적(사용자 확정): **로그를 터미널보다 편하게 스크롤하고 확인**하는 것. 캐릭터 시스템을
  VSCode로 통째로 이식하는 독립 확장안은 검토 후 보류 — 부록 A 참조.
- 근거 문서: `cli-control-design.md`, `session-log-design.md`, `subsystem-b-office.md`

## 1. 설계 원칙

- **앱이 정본, 확장은 뷰어.** 세션·캐릭터·알림·로그의 소유자는 앱이다. 확장은 읽고 보여줄 뿐,
  상태를 만들지 않는다. (쓰기는 알림 클리어와 선택적 응답 입력 정도로 한정.)
- **이미 있는 것만으로 시작한다.** 앱 쪽 코드 변경 없이 V1이 성립하도록 설계한다.
  - 로그: `<app_data>/session-logs/v1/<agentId>/<stamp>-<sid8>.log` — ANSI 제거·TUI 축약을
    거친 정리된 전사가 append-only로 이미 쌓이고 있다(설정 기본 켜짐, 30일+2GB GC).
  - 연결: control 서버(`cliEnabled` + control-token) — `/v1/list`가 캐릭터 이름·역할·상태·cwd를,
    `/v1/notifications`가 대기 알림을 준다.
- **로컬 전제.** 로그 파일 직접 읽기와 127.0.0.1 control 서버 모두 앱과 같은 기계의 로컬
  VSCode 창에서만 성립한다. Remote SSH 창 지원은 V3(앱에 로그 라우트 추가)로 미룬다.

## 2. 아키텍처

```
┌─ agent-office 앱 (그대로) ────────────────────────────────┐
│  control 서버  127.0.0.1:<control-port>, X-Agent-Office-Token │
│  세션 로그     <app_data>/session-logs/v1/<agentId>/*.log     │
│  프로필        <app_data>/profiles.json                       │
└──────────────┬────────────────────────────────────────────┘
     HTTP 폴링 │ /v1/list · /v1/notifications (1~2s)
   파일 읽기/watch │ 로그 · profiles.json (읽기 전용)
┌──────────────▼────────────────────────────────────────────┐
│  VSCode 확장                                                │
│  Discovery    app_data 자동발견(ctl과 동일 규약:              │
│               설정값 > AGENT_OFFICE_APP_DATA > OS 기본 경로), │
│               control-port/control-token 파일 읽기            │
│  Poller       list+notifications 폴링 → 트리 갱신·배지        │
│  TreeView     캐릭터 목록: 픽셀 얼굴 아이콘 + 상태 + 알림 배지 │
│  LogProvider  세션 로그 가상 문서(읽기 전용) + fs.watch tail   │
│  StatusBar    대표 캐릭터 1명 요약(선택)                       │
└───────────────────────────────────────────────────────────┘
```

- **연결 승인은 앱의 기존 2단계 옵트인 재사용**: 설정에서 `cliEnabled` ON → 앱에서 승인해
  control-token(0600) 발급. 확장은 `ctl`과 동일하게 포트·토큰 파일을 읽는 또 하나의
  클라이언트일 뿐이다. 파일이 없으면 확장이 안내 메시지로 승인 절차를 알려준다.
- **폴링으로 충분**: 알림 push 채널이 없으므로 1~2초 폴링. 상태 표시용이라 지연 허용.
  앱 미실행(연결 실패) 시 트리에 "앱 꺼짐" 표시로 강등, 로그 파일 열람은 계속 가능.

## 3. 기능 상세

### V1 — 로그 뷰어 (앱 무변경)

1. **캐릭터 트리 뷰** (활동바 컨테이너): `/v1/list` 기반 목록. 상태별 아이콘/설명
   (`running`=작업중, 알림 pending=⚠ 배지+정렬 상단, 없음=유휴), `profiles.json`에서
   역할·이름 보강. 뷰 배지에 총 대기 알림 수.
2. **세션 로그 열람**: 캐릭터 노드 펼침 → 세션 로그 파일 최근순 목록(파일명의
   `<stamp>-<sid8>` 파싱) → 클릭 시 열기.
   - **tail-follow 가상 문서**(기본): `TextDocumentContentProvider` 읽기 전용 문서 +
     `fs.watch`로 append 반영. 커서가 문서 끝에 있을 때만 자동 스크롤(위로 스크롤해
     읽는 중이면 방해하지 않음 — 이 UX가 이 확장의 존재 이유다).
   - **파일로 열기**(보조): 그냥 해당 .log를 에디터로 여는 명령(외부 도구 연계용).
   - 대용량 대비: 마지막 N KB(기본 512KB)부터 로드 + "전체 불러오기" 액션.
3. **알림 액션**: 알림 배지 클릭 → 메시지 표시 + `/v1/clear`. 설정으로 OS 알림 토글.
4. **연결 상태 표시 + 온보딩**: 미연결 사유(앱 꺼짐 / cliEnabled OFF / 미승인 /
   sessionLogEnabled OFF)를 구분해 안내.

### V2 — 캐릭터 정체성 + 편의

1. **픽셀 얼굴 아이콘**: `sheetGen.ts`(Pixi 비의존 순수 모듈)를 가져와 seed+archetype으로
   트리 아이콘용 16px 얼굴 PNG를 결정적 생성 — 앱과 같은 외형. 커스텀 스프라이트는
   `<app_data>/sprites/<agentId>.png`에서 idle0 프레임 추출.
2. **응답 입력**(선택): 알림에 "답장" 액션 → 입력 상자 → `/v1/send`로 세션 stdin 주입.
3. **대화록 뷰**(선택): `claude-resume.json`의 transcriptPath로 원본 JSONL을 읽어
   마크다운 렌더(웹뷰) — 정리된 대화 형태로 보고 싶을 때.
4. **상태바 대표 캐릭터**: `pickMascotTarget` 규칙 축소판(알림 최신 → working 최신).

### V3 — 원격/실시간 (앱 변경 필요, 필요해지면)

- control 서버에 `/v1/logs/list`·`/v1/logs/tail`(offset 기반) 라우트 추가 → Remote SSH
  창에서도 동작. 알림 long-poll 또는 SSE. 이때부터 파일 직접 읽기 의존 제거 가능.

## 4. 코드 공유

- 독립 이식안의 모노레포 전환(M0)은 **불필요**. 공유가 필요한 것은 V2의 `sheetGen` 계열
  (sheetGen·archetypes·parts·palette·prng·compositor, 전부 순수 모듈) 정도다.
- V1은 공유 코드 없음 → 저장소에 `vscode-ext/` 폴더 하나로 시작한다.
- V2에서 sheetGen을 쓸 때 결정: 소규모 `packages/character-gen` 추출(시드 결정성 계약을
  한 벌로 유지, 권장) vs 파일 복사. 확장에서 시드 외형이 앱과 달라 보이면 안 되므로
  추출 쪽이 안전하다.

## 5. 마일스톤·완료 기준

| 단계 | 내용 | 완료 기준 |
|---|---|---|
| **V1** | 트리 뷰 + 폴링 + tail-follow 로그 문서 + 온보딩 | 앱 실행 중 VSCode에서 캐릭터 선택 → 로그를 스크롤·검색하고, 새 출력이 자동 반영되며, 위로 스크롤해 읽는 동안 튀지 않는다 |
| **V2** | 픽셀 얼굴 아이콘 + 알림 답장 + 상태바 | 트리에서 앱과 같은 얼굴이 보이고, 알림에 VSCode 안에서 응답 가능 |
| **V3** | 로그 라우트/SSE (원격) | Remote SSH 창에서 V1 동등 기능 |

## 6. 주의점

- **로그 전사의 성격**: session-log는 2초 틱 tail + TUI 재그리기 축약이라 실시간 터미널
  미러가 아니라 "정리된 대화록"이다. 스크롤 열람 목적에는 장점이지만, 실시간성이 필요한
  장면은 앱 터미널을 계속 쓴다는 전제를 문서·온보딩에 명시한다.
- **토큰 취급**: control-token은 파일에서 읽어 메모리에만 보관, 확장 설정/로그에 남기지
  않는다. `cliEnabled`를 확장이 켜려 들지 않는다(앱의 권한 상승 차단 설계 존중).
- **폴링 예산**: 창이 백그라운드면 폴링 간격을 늘린다(예: 10s). 앱 종료 감지 시 중단.
- **파일명 규약 의존**: `<stamp>-<sid8>.log` 파싱은 `session_log/store.rs`가 정본 —
  바뀌면 확장도 따라가야 한다. V3 라우트가 생기면 이 의존은 사라진다.

## 7. 미결정 사항

1. 마켓플레이스 공개 여부. (이름은 V1 구현에서 `agent-office-logs`/"Agent Office 로그"로
   일단 확정 — 공개 시 재검토.)
2. V2 응답 입력(`/v1/send`)을 넣을지 — 뷰어 순수성 vs 편의.
3. V3 착수 조건(Remote SSH 창을 실제로 쓰게 되는 시점).

---

## 부록 A — 독립 이식안 검토 결과 (보류)

앱 없이 캐릭터 시스템 전체를 VSCode 확장으로 이식하는 안을 먼저 설계했으나, 사용자
목적이 "로그 열람"으로 확인되어 보류했다. 조사에서 확정한 사실은 장래를 위해 남긴다
(전체 설계는 이 문서의 git 이력 76bf252 판 참조):

- 캐릭터 서브시스템의 입력 경계는 이벤트 3종(`session-state`/`notification-*`/`activity-event`),
  출력 경계는 콜백 3종(`agentClicked`/`agentHover`/`deskClicked`)뿐이다.
- 캐릭터는 PTY에 직접 의존하지 않는다 — 외부 세션 attach가 증명. VSCode 터미널 구조와 동일 상황.
- `gen/**`·behaviorFsm·turnReducer·labelText·deskAssignment·subagentCounts는 순수 모듈로 즉시 이식 가능.
- 이식 시 재구현 대상은 훅 수신 서버+forwarder, settings.json/env 주입, Pixi→캔버스 2D 렌더러 3가지.
- 함정: 훅 커맨드에 포트 고정 금지(포트 파일 재시도), `background_tasks` 카운트의 `agent_id`
  조건과 워터마크, 서브에이전트 `Stop`/`UserPromptSubmit` 폐기, `sh_quote` 인용.
