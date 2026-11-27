// src/shared/types/link.ts
//
// 머신 연동(tailnet·LAN 안의 앱끼리 호스트-클라이언트). Rust `link::*` 미러.

/** 끔·호스트·클라이언트 중 하나. 클라이언트는 호스트 하나에만 붙고, 호스트는 다른 호스트에 붙지 못한다. */
export type LinkMode = "off" | "host" | "client";

/** `link.json`. */
export interface LinkConfig {
  mode: LinkMode;
  /** 클라이언트가 붙을 호스트 주소(정리한 모양, 예 `https://192.168.0.5:7788`). 비면 붙지 않는다. */
  hostAddress: string;
}

/** 호스트에 페어링된 기기 하나. */
export interface LinkDevice {
  id: string;
  name: string;
  machineId: string;
  /** epoch ms. */
  pairedAt: number;
  lastSeenAt?: number;
  appVersion?: string;
  connected: boolean;
  connectedAt?: number;
  /** 붙어 있는 연결의 상대 주소. */
  address?: string;
}

/** 호스트가 띄운 페어링 코드. */
export interface LinkPairing {
  code: string;
  /** 만료(epoch ms). */
  expiresAt: number;
}

export interface LinkHostStatus {
  pairing?: LinkPairing;
  devices: LinkDevice[];
}

export type LinkClientState = "idle" | "connecting" | "connected" | "waiting" | "rejected";

export interface LinkClientStatus {
  state: LinkClientState;
  paired: boolean;
  hostName?: string;
  deviceId?: string;
  address?: string;
  /** LAN 주소로 페어링했다면 고정한 호스트 CA 지문. */
  pinnedFingerprint?: string;
  /** 이유 코드(`link-unauthorized`·`link-protocol-mismatch`·`link-unreachable: …` 등). */
  error?: string;
  hostProtocol?: number;
  hostAppVersion?: string;
  /** 다시 붙어 볼 때(epoch ms, waiting일 때만). */
  retryAt?: number;
  connectedAt?: number;
  /** 아직 호스트가 받지 않은 이벤트·보고 수. */
  pending: number;
}

/** `link_status` 응답·`link-status` 이벤트. */
export interface LinkStatus {
  config: LinkConfig;
  /** 이 앱의 연동 프로토콜 버전(앱 버전과 별개, 같아야 붙는다). */
  protocol: number;
  appVersion: string;
  machineId: string;
  /** 호스트 모드일 때만. */
  host?: LinkHostStatus;
  /** 클라이언트 모드일 때만. */
  client?: LinkClientStatus;
}

/** 호스트 오피스의 머신별 층에 서는 원격 캐릭터 하나. Rust `link::remote::RemoteCharacter`. */
export interface RemoteCharacter {
  /** `기기id:캐릭터id`. 대화 보기 커맨드(`chat_view_*`)에 그대로 넘기면 그 머신에 묻는다. */
  id: string;
  /** 그 머신에서의 캐릭터 id. */
  agentId: string;
  name: string;
  role: string;
  /** 그 머신에서의 시작 폴더(그 머신 경로). 없으면 없다. */
  cwd?: string;
  /** 외형(`archetype`·`seed`·`colors`, 커스텀 시트·미니미 판). 판이 있으면 그림은 `load_sprite`·`load_minimi`에
   *  이 `id`를 넘겨 그 머신에 묻는다(`link/remoteImages.ts`). 없으면 null. */
  appearance: {
    archetype?: string | null;
    seed?: string;
    colors?: Record<string, string> | null;
    spriteUpdatedAt?: number | null;
    minimiUpdatedAt?: number | null;
  } | null;
  /** 초상 판(epoch ms). 없으면 초상 없음. 그림은 `load_portrait`에 이 `id`를 넘겨 묻는다. */
  portrait?: number;
  /** 세션 상태. 세션이 없거나 머신이 끊겼으면 없다. */
  sessionState?: "starting" | "running" | "exited" | "disposed";
  /** 권한 요청이 걸렸거나 턴이 사람을 기다림. */
  needsHuman: boolean;
  clockedOut: boolean;
}

export interface RemoteMachine {
  deviceId: string;
  name: string;
  /** 끊겼으면 false — 마지막으로 본 명단만 꺼진 모습으로 보인다. */
  online: boolean;
  characters: RemoteCharacter[];
  /** 그 머신에 클론이 있는 저장소(정규화한 git 원격 주소, 예 `github.com/acme/app`). 끊겨도 마지막 목록. */
  repos: string[];
}

/** 호스트가 원격 캐릭터 머신에 묻는 업무 일(`link_work`). */
export type LinkWorkKind =
  | "work-send" | "work-followup" | "work-reply" | "usage"
  // 팀 토의(Rust `link::discuss`): 호스트가 진행하는 토의의 그 머신 참가자 세션.
  | "discuss-open" | "discuss-turn" | "discuss-collect" | "discuss-state" | "discuss-close";

/** 클라이언트: 호스트가 이 머신에 연(`opened`)·닫은 토의 참가자 세션(Rust `link::discuss::EVENT`). 이 머신은 탭으로 보이기만 한다. */
export type LinkDiscussionSessionEvent =
  | {
      opened: true;
      /** 토의 세션 agentId(`discuss-<토의 id>.<이 머신 캐릭터>`). */
      agentId: string;
      sessionId: string;
      discussionId: string;
      /** 이 머신 캐릭터 id. */
      participantAgentId: string;
      cli: import("./discussion").DiscussionParticipant["agent"]["cli"];
      collection: import("./discussion").DiscussionCollection;
      observed: boolean;
      submittedAt: number;
      hostName: string;
      topic: string;
    }
  | { opened: false; agentId: string };

/** 그 머신 토의 세션의 지금(Rust `link::discuss` `discuss-state`). 시각은 그 머신 시계다. */
export interface LinkDiscussionState {
  alive: boolean;
  turn: { phase: "working" | "needs-human" | "idle" | "ended"; at: number } | null;
  /** `after` 뒤의 턴 토큰(Rust `control::turns::DiscussionUsage`). */
  usage: { seq: number; at: number; tokens: import("./session").SessionEventTokens; partial: boolean }[];
}

/** 클라이언트가 올리는 받은 업무 하나의 진행 상태(`link_task_report`). 끊겨 있으면 쌓였다가 최신 하나가 간다. */
export interface LinkTaskReport {
  /** 호스트 보드의 업무 id(받은 업무도 같은 id). */
  taskId: string;
  status: "todo" | "in_progress" | "review" | "done";
  /** 최신 세션(그 머신의 세션 id). */
  sessionId?: string;
  /** 그 세션의 지금 단계. */
  phase?: "working" | "waiting" | "review" | "ended";
  /** 단계가 턴 전이(프롬프트·턴 끝)인가. */
  transition?: boolean;
  /** 보내기 실패 이유 코드. */
  error?: string;
  /** 호스트가 붙인 보내기 번호. 어느 보내기의 결과인지 가른다. */
  sendSeq?: number;
}

/** 호스트 `link-task` 이벤트. */
export interface LinkTaskEvent extends LinkTaskReport {
  deviceId: string;
  /** 호스트가 받은 시각(호스트 시계, epoch ms). */
  receivedAt: number;
}

/** `link_remote` 응답·`link-remote` 이벤트(호스트 모드에서만 채워진다). */
export interface RemoteOffice {
  machines: RemoteMachine[];
}
