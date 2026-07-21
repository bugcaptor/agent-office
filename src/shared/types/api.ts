// src/shared/types/api.ts
//
// Domain slice: the renderer-facing AgentOfficeApi surface itself.
// See src/shared/types.ts for the frozen-contract overview.

import type { AgentId } from './common';
import type {
  CreateSessionOptions,
  CreateSessionResult,
  SessionStateEvent,
  SessionTurnRecord,
  SessionEventRecord,
  AdoptedSessionInfo,
  AvailableShell,
  ClaudeResumeEntry,
} from './session';
import type { NotificationEvent, ActivityEvent } from './notification';
import type { PersistedState, GeneratedSpriteImage } from './profile';
import type {
  SummaryProvider,
  SummaryPurpose,
  GetAppSettingsResult,
  AppSettings,
  ControlStatus,
} from './settings';
import type { BotAgentStatus, BotStatus } from './bot';
import type { DiaryEntry, WorkLogItem } from './diary';
import type { UsageSnapshot } from './usage';
import type {
  MarkdownListResult,
  MarkdownReadResult,
  MarkdownWriteResult,
} from './markdown';
import type {
  WorkdirListResult,
  WorkdirSearchResult,
  GitStatusResult,
  GitDiffResult,
  GitDiffMode,
  GitFileHistoryResult,
  GitCommitFilesResult,
} from './git';

/**
 * Renderer-facing API surface (frozen). Implemented by
 * `src/renderer/ipc/tauriApi.ts` via Tauri commands (invoke) + events
 * (listen) + a dedicated output `Channel` (exact command/event names are
 * in `src/shared/ipc.ts`).
 *
 * sessionId is a Rust-backend-internal concept (hook routing, settings file
 * naming) and never crosses this boundary — every method here is keyed by
 * `agentId`.
 */
export interface AgentOfficeApi {
  createSession(agentId: string, opts?: CreateSessionOptions): Promise<CreateSessionResult>;
  disposeSession(agentId: string): Promise<void>;
  /** fire-and-forget */
  writeInput(agentId: string, data: string): void;
  resize(agentId: string, cols: number, rows: number): void;
  clearNotifications(agentId: string, ids?: string[]): void;
  listNotifications(agentId: string): Promise<NotificationEvent[]>;
  loadState(): Promise<PersistedState>;
  saveState(state: PersistedState): Promise<void>;
  setBadgeCount(n: number): void;
  /** 초상 PNG(base64, data: prefix 없음) 저장. */
  savePortrait(agentId: string, pngBase64: string): Promise<void>;
  /** 초상 base64를 반환, 파일 없으면 null. */
  loadPortrait(agentId: string): Promise<string | null>;
  /** 초상 파일 삭제(없어도 성공). */
  deletePortrait(agentId: string): Promise<void>;
  /** 커스텀 스프라이트 시트 PNG(base64, data: prefix 없음, 64×16) 저장. */
  saveSprite(agentId: string, pngBase64: string): Promise<void>;
  /** 스프라이트 시트 base64를 반환, 파일 없으면 null. */
  loadSprite(agentId: string): Promise<string | null>;
  /** 스프라이트 파일 삭제(없어도 성공). */
  deleteSprite(agentId: string): Promise<void>;
  /** 머리 위 라벨 요약: 캡처한 provider의 로컬 CLI를 호출한다. 호출마다 사용자 구독/크레딧을 소모할 수 있다.
   *  purpose로 백엔드 타임아웃을 고른다(#66) — 기본 "label"(20초), 일기는 "diary"(120초). */
  summarizeText(
    provider: SummaryProvider,
    instruction: string,
    text: string,
    purpose?: SummaryPurpose,
  ): Promise<string>;
  /** PixelLab로 64×64 스프라이트 1장 생성. 동기 HTTP — 수십 초 걸릴 수 있다. */
  generateSpriteImage(description: string): Promise<GeneratedSpriteImage>;
  /** 앱 전역 opt-in 설정 로드. 인자 없음. */
  getAppSettings(): Promise<GetAppSettingsResult>;
  /** 앱 전역 opt-in 설정 저장. */
  setAppSettings(settings: AppSettings): Promise<void>;
  /** CLI 제어(#55) 상태 조회 — 서버 기동·승인 여부·포트·app_data 경로. */
  controlStatus(): Promise<ControlStatus>;
  /** CLI 제어를 승인(토큰 발급). 2단계 옵트인의 2단계. */
  controlApprove(): Promise<void>;
  /** CLI 제어 승인 취소(토큰 폐기). 이후 모든 CLI 요청 401. */
  controlRevoke(): Promise<void>;
  /** 봇 모드 시작(이슈 #57) — 이 탭의 폴링 태스크를 띄우고 로컬 입력을 봇에
   * 넘긴다. tea 미로그인/저장소 미감지 시 error가 채워진 상태를 반환. */
  botStart(agentId: string): Promise<BotAgentStatus>;
  /** 봇 모드 중단 — 폴링 태스크를 내리고 로컬 조작으로 복귀. */
  botStop(agentId: string): Promise<void>;
  /** 봇 모드가 켜진 탭들의 상태 스냅샷. */
  botStatus(): Promise<BotStatus>;
  /** 사용 가능한 셸 목록. Windows 외 플랫폼은 빈 배열. */
  listAvailableShells(): Promise<AvailableShell[]>;
  /** 디렉터리를 Visual Studio Code로 연다. VS Code 미설치/경로 부재 시 reject. */
  openInVscode(path: string): Promise<void>;
  /** 디렉터리를 OS 기본 터미널 앱으로 연다. 경로 부재/실행 실패 시 reject. */
  openInTerminal(path: string): Promise<void>;
  /** 셸 출력(터미널 버퍼 plain text)을 임시 .txt로 쓰고 설정한 외부 에디터로
   * 연다. 쓴 파일의 절대 경로를 반환, 쓰기/실행 실패 시 reject. */
  exportTerminalOutput(agentName: string, content: string): Promise<string>;
  /** 네이티브 폴더 선택 다이얼로그. 선택한 절대 경로, 취소 시 null.
   * `initialDir`이 실존 디렉터리면 거기서 시작한다(`~` 확장 포함). */
  pickDirectory(initialDir?: string): Promise<string | null>;
  /** Returns an unsubscribe function. `bytes` is the raw stream byte count of
   * this batch (§#49); the renderer accumulates it on write to derive snapshot
   * offsets. Restore snapshots deliver `bytes === 0`. */
  onData(agentId: string, cb: (data: string, bytes: number) => void): () => void;
  onSessionState(cb: (e: SessionStateEvent) => void): () => void;
  onNotification(cb: (n: NotificationEvent) => void): () => void;
  onNotificationCleared(cb: (p: { agentId: string; ids: string[] }) => void): () => void;
  /** activity-event(prompt/tool) 구독. Returns an unsubscribe function. */
  onActivity(cb: (e: ActivityEvent) => void): () => void;
  /** 완료된 턴 1건을 로컬 시계열 로그에 append (fire-and-forget). */
  appendSessionTurn(record: SessionTurnRecord): void;
  /** 누적된 세션 턴 기록 전체를 읽는다(통계용). 손상된 줄은 건너뛴다. */
  loadSessionTurns(): Promise<SessionTurnRecord[]>;
  /** 캐릭터 일기(#56) 한 편을 per-agent 로그에 append. 생성은 렌더러가
   * summarizeText로 이미 마친 상태 — 여기선 저장만 한다. */
  appendDiaryEntry(agentId: string, entry: DiaryEntry): Promise<void>;
  /** 한 캐릭터의 일기 전체(작성순)를 읽는다(열람 오버레이용). 손상된 줄은 건너뛴다. */
  loadDiary(agentId: string): Promise<DiaryEntry[]>;
  /** 캐릭터 일기(#60) 작업 로그 버퍼 전체를 스냅샷 저장한다. `items`가 비면
   * 스냅샷 파일을 삭제한다(일기화로 소진된 캐릭터). 렌더러가 디바운스로 호출. */
  saveWorkLog(agentId: string, items: WorkLogItem[]): Promise<void>;
  /** 전 캐릭터의 작업 로그 스냅샷을 읽는다(부팅 복원용). 손상/부재는 건너뛴다. */
  loadWorkLogs(): Promise<Record<string, WorkLogItem[]>>;
  /** 세션 이벤트 시계열에서 `fromAt..=toAt`(epoch ms) 범위를 읽는다(분석 패널용).
   * 없는 파일·손상 줄은 건너뛰며 항상 성공한다. `(at, runId, seq)` 정렬. */
  loadSessionEvents(fromAt: number, toAt: number): Promise<SessionEventRecord[]>;
  /** 세션 핸드오프(unix 전용) 지원 여부. Windows 등 미지원 플랫폼은 false. */
  handoffSupported(): Promise<boolean>;
  /** 종료 시 살아있는 세션들을 `sessiond` 데몬으로 넘긴다. `snapshots`는
   * agentId -> 직렬화된 터미널 화면(스크롤백 포함, xterm SerializeAddon
   * 출력) -- 데몬이 핸드오프 이전 화면을 보관할 방법이 이것뿐이므로 실어
   * 보낸다. `renderedBytes`(agentId -> 렌더러가 실제 렌더한 raw 스트림 바이트
   * 누적치)로 스냅샷 offset(=base+누적치)을 확정해 재입양 시 유실을 없앤다(§#49).
   * 넘긴 세션 수를 반환. */
  handoffSessions(
    snapshots: Record<string, string>,
    renderedBytes: Record<string, number>
  ): Promise<number>;
  /** 부팅 시 1회 — 데몬에 남아있던 세션을 되찾는다. 미지원/데몬 없음이면 빈 배열. */
  adoptDetachedSessions(): Promise<AdoptedSessionInfo[]>;
  /** v2 상시 브로커 모드(docs/session-broker-v2-design.md)가 켜져 있는지.
   * true일 때만 렌더러가 주기 스냅샷 업로드를 활성화한다. 미지원/기본은 false. */
  sessionBrokerMode(): Promise<boolean>;
  /** 브로커 모드 주기 스냅샷 업로드 — agentId -> 직렬화된 xterm 화면. 데몬이
   * 세션별 최신 것만 보관해 앱 크래시 후 화면 복원에 대비한다. 브로커 모드가
   * 아니거나 데몬에 못 닿으면 백엔드에서 no-op. `renderedBytes`(agentId ->
   * 렌더러가 실제 렌더한 raw 스트림 바이트 누적치)로 스냅샷 offset을 확정한다(§#49). */
  uploadSessionSnapshots(
    snapshots: Record<string, string>,
    renderedBytes: Record<string, number>
  ): Promise<void>;
  /** Claude 세션 이어하기 후보 목록(agentId → 최신 1건). 메뉴를 열 때 조회한다.
   * 캡처된 적 없는 에이전트는 키가 없다(빈 객체 가능). */
  listClaudeResumeSessions(): Promise<Record<AgentId, ClaudeResumeEntry>>;
  /** 구독 사용량(rate limit) 스냅샷을 홈 디렉터리 로컬 캐시에서 읽는다(인자 없음).
   * 파싱 실패한 provider는 null이며 호출 자체는 항상 성공한다. */
  loadUsageSnapshot(): Promise<UsageSnapshot>;
  /** `root` 하위의 마크다운(.md) 파일 목록(이슈 #10). 상한 초과 시 `truncated=true`. */
  markdownListFiles(root: string): Promise<MarkdownListResult>;
  /** `root` 기준 `relPath` 파일 내용과 버전을 읽는다. 부재/범위 밖이면 reject. */
  markdownReadFile(root: string, relPath: string): Promise<MarkdownReadResult>;
  /** `expectedVersion`이 현재 버전과 다르면 "CONFLICT"로 시작하는 메시지로 reject.
   * 성공 시 갱신된 버전을 돌려준다. */
  markdownWriteFile(
    root: string,
    relPath: string,
    content: string,
    expectedVersion: string,
  ): Promise<MarkdownWriteResult>;
  /** `root` 하위의 전체 파일 목록(이슈 #11, .gitignore 존중·hidden 스킵).
   * 상한(5000) 초과 시 `truncated=true`. */
  workdirListFiles(root: string): Promise<WorkdirListResult>;
  /** `root` 아래에서 `query`와 일치하는 파일을 서버사이드(Everything)로
   * 검색한다(이슈 #67) — `workdirListFiles`가 5000개 상한에 걸려 잘랐더라도,
   * 이 검색은 인덱스를 다시 훑어 상한 밖 파일도 찾아낼 수 있다. Walker
   * 백엔드/빈 쿼리/es.exe 실패는 모두 `usedIndex=false` + 빈 `files`로
   * 조용히 답한다(에러가 아니다) — 호출부는 이 신호로 기존 클라이언트 fuzzy
   * 필터로 되돌아가야 한다. */
  workdirSearchFiles(root: string, query: string): Promise<WorkdirSearchResult>;
  /** `root`의 git 상태(porcelain v2). 저장소 아님/타임아웃은 reject가 아니라
   * `isRepo=false`/`timedOut=true` 필드로 표현한다. */
  workdirGitStatus(root: string): Promise<GitStatusResult>;
  /** `root` 기준 `relPath`의 diff를 `mode` 관점으로 조회(이슈 #11 후속).
   * 미추적 파일은 `mode="untracked"`. 타임아웃은 `timedOut=true`로 표현. */
  workdirDiffFile(root: string, relPath: string, mode: GitDiffMode): Promise<GitDiffResult>;
  /** `root` 기준 `relPath`의 커밋 히스토리(`git log --follow`, 페이지네이션). */
  workdirFileHistory(
    root: string,
    relPath: string,
    limit: number,
    skip: number,
  ): Promise<GitFileHistoryResult>;
  /** 특정 커밋이 `relPath`에 만든 변경(diff). `commit`은 hex 7~40자. */
  workdirDiffCommit(root: string, commit: string, relPath: string): Promise<GitDiffResult>;
  /** 한 커밋이 바꾼 파일 목록(이슈 #54, 페이지네이션). 병합 커밋은 combined
   * diff라 목록이 빌 수 있다. */
  workdirCommitFiles(
    root: string,
    commit: string,
    limit: number,
    skip: number,
  ): Promise<GitCommitFilesResult>;
  /** 저장소 전체 커밋 로그(이슈 #54, 파일 지목 없음). `allBranches`면 모든
   * 참조를, `query`가 있으면 커밋 메시지를 대소문자 무시·부분일치로 필터한다. */
  workdirRepoLog(
    root: string,
    limit: number,
    skip: number,
    allBranches: boolean,
    query: string,
  ): Promise<GitFileHistoryResult>;
  /** 외부 비교 도구(`git difftool`)를 fire-and-forget으로 띄운다. `commit`이
   * 지정되면 그 커밋의 변경을, 아니면 `mode`의 현재 변경을 연다. 미설정 도구는
   * 백그라운드에서 조용히 실패(인앱 diff가 폴백). */
  workdirDifftool(
    root: string,
    relPath: string,
    mode: GitDiffMode,
    commit?: string,
  ): Promise<void>;
}
