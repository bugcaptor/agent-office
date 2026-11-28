// src/renderer/chatView/ChatView.tsx
//
// 터미널 탭의 "대화 보기". xterm은 아래에 그대로 붙어 있고(keep-alive), 이 판이 그 위를 덮는다.
// 내용은 CLI 전사(JSONL)다 — 사람 말·에이전트 말(마크다운)·도구 호출과 결과(접힘)·이미지.
// 권한 요청·질문은 카드로 뜨고 여기서 답할 수 있다. 터미널에도 같은 대화상자가 떠 있어
// 어느 보기에서든 먼저 답한 쪽이 반영된다. 위 입력칸은 CLI가 입력을 기다리면 같은 세션에 바로 잇고,
// 작업 중이면 대기열(웹 상태판과 공유)에 쌓아 턴이 끝날 때 넣는다.
// 웹 상태판처럼 최신이 맨 위다 — 블록 순서만 뒤집고, 도구 결과·이미지는 그 호출 블록 안에서 아래로
// 붙는다(서브에이전트 블록도 같다). 맨 아래를 따라가는 자동 스크롤은 이미지처럼 뒤늦게 높이가 느는
// 항목에서 자꾸 풀려서 버렸다. 맨 위에 있으면 새 항목이 그대로 보이고, 아래로 내려 읽는 중이면
// 브라우저 스크롤 앵커링이 읽던 자리를 지킨다. 최신 블록 위에는 지금 상태를 보인다 —
// 생각 중(움직임)과 출력이 끊긴 멈춤·입력 대기(정지)를 가른다.
import { createContext, memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AskAction, ChatLiveStatus, ChatLogItem, ChatLogPage, PendingAsk, RemoteSessionOp } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { ChatMarkdown, ChatPlainText, CopyableText, CopyButton, DownloadButton, ImageActions, type ShowNotice } from "./ChatLogActions";
import {
  askQuestions,
  groupChatItems,
  joinAnswers,
  liveState,
  mergeChatPage,
  minSec,
  permissionSummary,
  runningTool,
  type ChatBlock,
  type LiveState,
} from "./chatViewModel";
import { useAgentThumb } from "../portrait/agentThumb";
import "./chatView.css";

const POLL_MS = 1500;
/** 위 끝에서 이만큼 안쪽이면 "최신을 보고 있다"로 보고, 새 항목이 와도 맨 위에 붙여 둔다. */
const TOP_PX = 8;

/** 에이전트 말 옆에 붙일 캐릭터 얼굴 그림. 일반 에이전트(캐릭터 없음)는 undefined라 🤖를 쓴다. */
const SpeakerThumb = createContext<string | undefined>(undefined);

/** 말한 쪽 표시: 에이전트는 캐릭터 얼굴, 사람은 👤. */
function Speaker({ role }: { role: ChatLogItem["role"] }) {
  const thumb = useContext(SpeakerThumb);
  if (role === "assistant" && thumb) return <img className="chat-avatar" src={thumb} alt="" aria-hidden="true" />;
  return <span className="chat-avatar" aria-hidden="true">{role === "user" ? "👤" : "🤖"}</span>;
}

/** 이미지 data URL 캐시(탭을 오가도 다시 받지 않는다). null은 비워진 이미지. */
const imageCache = new Map<string, string | null>();

interface ImagePreview {
  id: string;
  src: string;
  opener: HTMLButtonElement;
}

function errorCode(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** 다른 머신 캐릭터(머신 연동 호스트)를 볼 때. 입력·답하기·대기열 빼기는 그 머신에 넘기고(`chat_view_*`가
 * `기기id:캐릭터id`를 알아본다), 머리에 세션 열기·이어하기·닫기·중단 단추가 붙는다. 끊긴 머신이면 보기만 한다. */
export interface RemoteChatTarget {
  /** 그 머신 이름(머리에 보인다). */
  machine: string;
  /** 그 머신이 붙어 있다(끊기면 조작을 막는다). */
  online: boolean;
  /** 그 캐릭터 세션 상태(`starting`·`running`·`exited`, 없으면 세션 없음). */
  sessionState?: string;
  onClose: () => void;
}

export function ChatView({ agentId, active, onShowTerminal, remote }: { agentId: string; active: boolean; onShowTerminal?: () => void; remote?: RemoteChatTarget }) {
  const { t } = useTranslation("terminal");
  const [page, setPage] = useState<ChatLogPage | null>(null);
  const [asks, setAsks] = useState<PendingAsk[]>([]);
  const [status, setStatus] = useState<ChatLiveStatus | null>(null);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [actionNotice, setActionNotice] = useState("");
  const [zoom, setZoom] = useState<ImagePreview | null>(null);
  const zoomImage = useRef<HTMLImageElement>(null);
  const zoomDialog = useRef<HTMLDivElement>(null);
  const noticeTimer = useRef<ReturnType<typeof setTimeout>>();
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const atTop = useRef(true);
  const pageRef = useRef<ChatLogPage | null>(null);
  pageRef.current = page;
  const remoteRef = useRef(remote);
  remoteRef.current = remote;
  // 끊긴 머신 캐릭터는 보기만 한다.
  const canAct = !remote || remote.online;

  const errText = (code: string) => t(`chatView.err.${code}`, { defaultValue: code });

  const showActionNotice = useCallback((message: string) => {
    clearTimeout(noticeTimer.current);
    setActionNotice(message);
    noticeTimer.current = setTimeout(() => setActionNotice(""), 2600);
  }, []);

  useEffect(() => () => clearTimeout(noticeTimer.current), []);
  useEffect(() => {
    if (zoom) zoomDialog.current?.focus({ preventScroll: true });
  }, [zoom]);

  const closeZoom = () => {
    setZoom(null);
    zoom?.opener.focus({ preventScroll: true });
  };

  // 보이는 동안만 묻는다. 탭을 떠나면 멈추고, 돌아오면 이어 받는다.
  useEffect(() => {
    if (!active) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      try {
        const prev = pageRef.current;
        const [next, pending, live] = await Promise.all([
          tauriApi.chatViewLog(agentId, prev?.last, prev?.epoch),
          tauriApi.chatViewAsks(agentId),
          tauriApi.chatViewStatus(agentId),
        ]);
        if (stop) return;
        setPage((p) => mergeChatPage(p, next));
        setAsks((a) => (JSON.stringify(a) === JSON.stringify(pending) ? a : pending));
        setStatus(live);
      } catch {
        // 앱이 바쁘거나 세션이 바뀌는 중 — 다음 틱에 다시 묻는다. 다른 머신이 끊겼으면 걸려 있던 요청은 내린다
        // (그 머신에 답할 수 없다).
        if (!stop && remoteRef.current && !remoteRef.current.online) setAsks((a) => (a.length ? [] : a));
      }
      if (!stop) timer = setTimeout(tick, POLL_MS);
    };
    void tick();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
  }, [agentId, active]);

  // 이 보기로 오면 키 입력이 아래 xterm으로 새지 않게 입력칸으로 포커스를 옮긴다.
  useEffect(() => {
    // 탭 활성화가 한 프레임 뒤 xterm에 포커스를 주므로(`TerminalRegistry.activate`) 그 다음 프레임에 가져온다.
    if (!active) return;
    let inner = 0;
    const outer = requestAnimationFrame(() => {
      inner = requestAnimationFrame(() => inputRef.current?.focus());
    });
    return () => {
      cancelAnimationFrame(outer);
      cancelAnimationFrame(inner);
    };
  }, [active]);

  const thumb = useAgentThumb(agentId);
  const blocks = useMemo(() => groupChatItems(page?.items ?? []), [page]);
  const live = liveState(status, runningTool(blocks));
  const queued = status?.queue.items ?? [];
  const queueError = status?.queue.error;
  // 작업 중이거나 앞서 쌓인 것이 있으면 보내기는 대기열에 쌓는다.
  // 단, Claude가 작업 중이고 답을 기다리는 대화상자가 없으면 바로 넣는다(다음 도구 호출 뒤에 반영).
  const steer = !!status?.queue.steer;
  const willQueue = !!status && ((status.phase === "working" && !steer) || status.phase === "needs-human" || queued.length > 0);
  const newestFirst = useMemo(() => [...blocks].reverse(), [blocks]);
  const liveKind = live ? live.kind : "";

  // 맨 위에서 보고 있으면 위로 새 항목이 붙어도 맨 위에 남긴다(앵커링이 아래로 밀지 않게).
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && atTop.current) el.scrollTop = 0;
  }, [newestFirst, asks, liveKind]);

  const refreshStatus = () => {
    tauriApi.chatViewStatus(agentId).then(setStatus, () => {});
  };

  // 작업 중이면 대기열을 건너뛰고 도는 턴을 끊어 바로 보낼 수 있다. 답을 기다리는 대화상자가 있으면 글이 거기 들어가 막는다.
  const canSendNow = !!status && status.phase === "working" && asks.length === 0;

  const send = async (now = false) => {
    const text = draft.trim();
    if (!text || sending) return;
    setSending(true);
    setNotice(null);
    try {
      const outcome = await tauriApi.chatViewSend(agentId, text, now);
      setDraft("");
      atTop.current = true;
      scrollRef.current?.scrollTo({ top: 0 });
      if (now) showActionNotice(t("chatView.sentNowNotice"));
      else if (outcome === "queued") showActionNotice(t("chatView.queuedNotice"));
      else if (outcome === "launched") showActionNotice(t("chatView.launchedNotice"));
      refreshStatus();
    } catch (err) {
      setNotice(errText(errorCode(err)));
    } finally {
      setSending(false);
    }
  };

  const cancelQueued = async (id: string) => {
    try {
      await tauriApi.chatViewCancelQueued(agentId, id);
    } catch (err) {
      setNotice(errText(errorCode(err)));
    }
    refreshStatus();
  };

  const answer = async (ask: PendingAsk, action: AskAction, answers?: Record<string, string>, message?: string) => {
    try {
      await tauriApi.chatViewAnswer(ask.id, action, answers, message, remote ? agentId : undefined);
      setAsks((a) => a.filter((x) => x.id !== ask.id));
      setNotice(null);
    } catch (err) {
      const code = errorCode(err);
      setNotice(errText(code));
      if (code === "ask-gone") setAsks((a) => a.filter((x) => x.id !== ask.id));
    }
  };

  return (
    <div className="chat-view" data-agent-id={agentId}>
      <div className="chat-view-bar">
        <span className="chat-view-bar-title">{remote ? t("chatView.remoteTitle", { machine: remote.machine }) : t("chatView.title")}</span>
        <span className="chat-view-bar-hint">{remote ? (remote.online ? t("chatView.remoteHint") : t("chatView.remoteOffline", { machine: remote.machine })) : t("chatView.hint")}</span>
        {remote
          ? <button type="button" className="pixel-btn" onClick={remote.onClose}>{t("chatView.close")}</button>
          : <button type="button" className="pixel-btn" onClick={onShowTerminal}>{t("chatView.showTerminal")}</button>}
      </div>
      {remote && remote.online && (
        <RemoteSessionBar
          agentId={agentId}
          sessionState={remote.sessionState}
          onDone={() => { setNotice(null); showActionNotice(t("chatView.remoteOpSent", { machine: remote.machine })); refreshStatus(); }}
          onError={(code) => setNotice(errText(code))}
        />
      )}
      {canAct && <div className="chat-view-input">
        <textarea
          ref={inputRef}
          value={draft}
          placeholder={willQueue ? t("chatView.inputQueuePh") : steer ? t("chatView.inputSteerPh") : t("chatView.inputPh")}
          rows={2}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // 조합 중(한글 IME)인 Enter는 글자 확정이다.
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
        />
        <button type="button" className="pixel-btn primary" disabled={sending || !draft.trim()} onClick={() => void send()}>
          {willQueue ? t("chatView.queue") : t("chatView.send")}
        </button>
        {canSendNow && (
          <button type="button" className="pixel-btn" disabled={sending || !draft.trim()} title={t("chatView.sendNowHint")} onClick={() => void send(true)}>
            {t("chatView.sendNow")}
          </button>
        )}
      </div>}
      {notice && <div className="chat-view-notice" role="status">{notice}</div>}
      {(queued.length > 0 || queueError) && (
        <div className="chat-queue" aria-label={t("chatView.queueTitle", { count: queued.length })}>
          {queued.length > 0 && <div className="chat-queue-title">{t("chatView.queueTitle", { count: queued.length })}</div>}
          {queued.map((m, i) => (
            <div key={m.id} className="chat-queue-item">
              <span className="chat-queue-n">{i + 1}</span>
              <span className="chat-queue-text" title={m.text}>{m.text}</span>
              {canAct && <button type="button" className="pixel-btn chat-queue-cancel" aria-label={t("chatView.queueCancel")} title={t("chatView.queueCancel")} onClick={() => void cancelQueued(m.id)}>✕</button>}
            </div>
          ))}
          {queueError && <div className="chat-queue-error">{errText(queueError)}</div>}
        </div>
      )}
      <div
        className="chat-view-scroll"
        ref={scrollRef}
        onScroll={(e) => {
          atTop.current = e.currentTarget.scrollTop < TOP_PX;
        }}
      >
        {asks.map((a) => <AskCard key={a.id} ask={a} onAnswer={answer} onNotice={showActionNotice} remote={!!remote} />)}
        {page?.attached && live && <LiveRow live={live} />}
        {!page ? (
          <div className="chat-view-empty">…</div>
        ) : !page.attached ? (
          <div className="chat-view-empty">{t("chatView.noLog")}</div>
        ) : !blocks.length ? (
          <div className="chat-view-empty">{t("chatView.noItems")}</div>
        ) : (
          <SpeakerThumb.Provider value={thumb}>
            {newestFirst.map((b) => (
              <Block key={`${page.epoch}/${b.key}`} block={b} agentId={agentId} epoch={page.epoch} onZoom={setZoom} onNotice={showActionNotice} />
            ))}
          </SpeakerThumb.Provider>
        )}
      </div>
      {zoom && (
        <div
          className="chat-view-zoom"
          role="dialog"
          aria-label={t("chatView.zoom")}
          tabIndex={-1}
          ref={zoomDialog}
          onClick={closeZoom}
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.stopPropagation();
            closeZoom();
          }}
        >
          <div className="chat-view-zoom-content">
            <img ref={zoomImage} src={zoom.src} alt="" />
            <ImageActions imageRef={zoomImage} agentId={agentId} imageId={zoom.id} onNotice={showActionNotice} />
          </div>
        </div>
      )}
      <div className="chat-action-notice" role="status" aria-atomic="true">{actionNotice}</div>
    </div>
  );
}

/** 최신 블록 위의 지금 상태 한 줄. 생각 중은 움직이고, 멈춤·입력 대기는 멈춰 있다. */
function LiveRow({ live }: { live: LiveState }) {
  const { t } = useTranslation("terminal");
  const time = (ms: number) => {
    const [m, s] = minSec(ms);
    return m ? t("chatView.live.minSec", { m, s }) : t("chatView.live.sec", { s });
  };
  let text: string;
  switch (live.kind) {
    case "thinking":
      text = live.tool ? t("chatView.live.tool", { tool: live.tool, time: time(live.elapsedMs) }) : t("chatView.live.thinking", { time: time(live.elapsedMs) });
      break;
    case "stalled":
      text = t("chatView.live.stalled", { time: time(live.silentMs) });
      break;
    case "human":
      text = t("chatView.live.human");
      break;
    case "idle":
      text = t("chatView.live.idle");
      break;
    case "off":
      text = t("chatView.live.off");
      break;
  }
  return (
    <div className={`chat-live ${live.kind}`} role="status">
      {live.kind === "thinking" ? <span className="chat-live-dots" aria-hidden="true"><i /><i /><i /></span> : <span className="chat-live-mark" aria-hidden="true" />}
      <span>{text}</span>
    </div>
  );
}

interface ItemActions {
  agentId: string;
  epoch: number;
  onZoom: (image: ImagePreview) => void;
  onNotice: ShowNotice;
}

// 항목이 2000개까지 쌓이므로 블록·항목은 memo로 감싼다. 상태 줄이 매초 바뀌어도, 새 항목이 붙어도
// 이미 그린 항목은 다시 그리지 않는다(`mergeChatPage`가 들고 있던 항목 객체를 그대로 둔다).
const Block = memo(function Block({ block, ...actions }: ItemActions & { block: ChatBlock }) {
  return (
    <div className={`chat-block${block.side ? " side" : ""}`}>
      <Item item={block.head} {...actions} />
      {block.children.map((c) => <Item key={c.seq} item={c} {...actions} nested />)}
    </div>
  );
});

const Item = memo(function Item({ item, agentId, epoch, onZoom, onNotice, nested = false }: ItemActions & { item: ChatLogItem; nested?: boolean }) {
  const { t } = useTranslation("terminal");
  if (item.kind === "image") return <ChatImage item={item} agentId={agentId} onZoom={onZoom} onNotice={onNotice} />;
  if (item.kind === "text") {
    return (
      <div className={`chat-row ${item.role}`}>
        <Speaker role={item.role} />
        <div className={`chat-msg ${item.role}`}>
          {item.role === "assistant"
            ? <ChatMarkdown text={item.text} onNotice={onNotice} />
            : <ChatPlainText text={item.text} onNotice={onNotice} />}
          {(item.text || !!item.files?.length) && (
            <div className="chat-actions">
              {item.text && <CopyButton text={item.text} message onNotice={onNotice} />}
              {item.files?.map((file, index) => (
                <DownloadButton
                  key={index}
                  agentId={agentId}
                  source={{ kind: "file", epoch, seq: item.seq, index }}
                  filename={file.name}
                  onNotice={onNotice}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    );
  }
  if (item.kind === "tool_use") {
    const first = item.text.split("\n").find((l) => l.trim()) ?? "";
    if (!item.toolName) return <div className="chat-tool plain">⤷ {first}</div>;
    return (
      <details className="chat-tool">
        <summary>🔧 {item.toolName}: {first}</summary>
        <CopyableText text={item.text} onNotice={onNotice} />
      </details>
    );
  }
  const lines = item.text.split("\n").length;
  const first = item.text.split("\n").find((l) => l.trim()) ?? "";
  return (
    <details className={`chat-tool${item.isError ? " err" : ""}${nested ? " nested" : ""}`}>
      <summary>↳ {t("chatView.result", { count: lines })} · {first.slice(0, 80)}</summary>
      <CopyableText text={item.text} onNotice={onNotice} />
    </details>
  );
});

function ChatImage({ item, agentId, onZoom, onNotice }: Omit<ItemActions, "epoch"> & { item: ChatLogItem }) {
  const { t } = useTranslation("terminal");
  const imageRef = useRef<HTMLImageElement>(null);
  const id = item.image?.id;
  const key = `${agentId}/${id}`;
  const [src, setSrc] = useState<string | null | undefined>(() => (id ? imageCache.get(key) : null));
  useEffect(() => {
    if (!id || imageCache.has(key)) return;
    let alive = true;
    tauriApi.chatViewImage(agentId, id).then(
      (url) => { imageCache.set(key, url); if (alive) setSrc(url); },
      () => { if (alive) setSrc(null); },
    );
    return () => { alive = false; };
  }, [agentId, id, key]);
  // 그림이 온 경로·URL은 그림과 함께 늘 보인다(그림을 더 볼 수 없어도).
  const source = item.image?.source;
  const sourceLine = source && <div className="chat-image-source" title={source}>{source}</div>;
  if (!id || src === null) return <div className="chat-tool plain">{t("chatView.imageGone")}{sourceLine}</div>;
  if (src === undefined) return <div className="chat-image loading" />;
  return (
    <div className={`chat-image ${item.role}`}>
      <button type="button" className="chat-image-open" onClick={(event) => onZoom({ id, src, opener: event.currentTarget })} aria-label={t("chatView.zoom")}>
        <img ref={imageRef} src={src} alt="" onError={() => setSrc(null)} />
      </button>
      {sourceLine}
      <ImageActions imageRef={imageRef} agentId={agentId} imageId={id} onNotice={onNotice} />
    </div>
  );
}

/** 다른 머신 캐릭터의 세션 단추. 세션이 없으면 열기·이어하기, 있으면 다시 열기·이어하기·중단·닫기.
 * 세션을 끊는 것(다시 열기·이어하기·닫기)은 한 번 더 눌러 확인한다. */
function RemoteSessionBar({ agentId, sessionState, onDone, onError }: {
  agentId: string;
  sessionState?: string;
  onDone: () => void;
  onError: (code: string) => void;
}) {
  const { t } = useTranslation("terminal");
  const [busy, setBusy] = useState(false);
  const [armed, setArmed] = useState<RemoteSessionOp | null>(null);
  const armTimer = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => () => clearTimeout(armTimer.current), []);
  const live = sessionState === "running" || sessionState === "starting";
  const run = async (op: RemoteSessionOp, confirm: boolean) => {
    if (busy) return;
    if (confirm && armed !== op) {
      setArmed(op);
      clearTimeout(armTimer.current);
      armTimer.current = setTimeout(() => setArmed(null), 3000);
      return;
    }
    setArmed(null);
    setBusy(true);
    try {
      await tauriApi.chatViewRemoteOp(agentId, op);
      onDone();
    } catch (err) {
      onError(errorCode(err));
    } finally {
      setBusy(false);
    }
  };
  const button = (op: RemoteSessionOp, label: string, confirm: boolean, cls = "pixel-btn") => (
    <button type="button" className={cls} disabled={busy} onClick={() => void run(op, confirm)}>
      {armed === op ? t("chatView.again") : label}
    </button>
  );
  return (
    <div className="chat-view-session" role="group" aria-label={t("chatView.sessionGroup")}>
      {live
        ? <>
            {button("interrupt", t("chatView.sessionInterrupt"), false)}
            {button("restart", t("chatView.sessionRestart"), true)}
            {button("resume", t("chatView.sessionResume"), true)}
            {button("terminate", t("chatView.sessionClose"), true, "pixel-btn danger")}
          </>
        : <>
            {button("restart", t("chatView.sessionOpen"), false, "pixel-btn primary")}
            {button("resume", t("chatView.sessionResume"), false)}
          </>}
    </div>
  );
}

function AskCard({ ask, onAnswer, onNotice, remote = false }: {
  ask: PendingAsk;
  /** 다른 머신의 요청(답은 그 머신 훅 중계로 간다). */
  remote?: boolean;
  onAnswer: (ask: PendingAsk, action: AskAction, answers?: Record<string, string>, message?: string) => Promise<void>;
  onNotice: ShowNotice;
}) {
  const { t } = useTranslation("terminal");
  const [busy, setBusy] = useState(false);
  const [picks, setPicks] = useState<Record<string, string[]>>({});
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [reason, setReason] = useState("");
  const [armed, setArmed] = useState(false);
  const run = async (action: AskAction, answers?: Record<string, string>, message?: string) => {
    if (busy) return;
    setBusy(true);
    try { await onAnswer(ask, action, answers, message); } finally { setBusy(false); }
  };
  const question = ask.toolName === "AskUserQuestion";
  const title = question ? t("chatView.askQuestion") : ask.toolName === "ExitPlanMode" ? t("chatView.askPlan") : t("chatView.askPermission", { tool: ask.toolName });

  if (question) {
    const qs = askQuestions(ask);
    const toggle = (q: (typeof qs)[number], label: string) => {
      setPicks((p) => {
        const cur = p[q.question] ?? [];
        const has = cur.includes(label);
        const next = q.multiSelect ? (has ? cur.filter((l) => l !== label) : [...cur, label]) : has ? [] : [label];
        return { ...p, [q.question]: next };
      });
      if (!q.multiSelect) setTyped((v) => ({ ...v, [q.question]: "" }));
    };
    const answers = joinAnswers(qs, picks, typed);
    const ready = qs.length > 0 && Object.values(answers).every(Boolean);
    return (
      <div className="chat-ask">
        <div className="chat-ask-title">{ask.sidechain ? "⤷ " : ""}{title}</div>
        {qs.map((q) => (
          <div key={q.question} className="chat-ask-q">
            {q.header && <span className="chat-ask-tag">{q.header}</span>}
            <div className="chat-ask-qt">{q.question}</div>
            <div className="chat-ask-opts">
              {q.options.map((o) => (
                <button
                  key={o.label}
                  type="button"
                  className={`chat-ask-opt${(picks[q.question] ?? []).includes(o.label) ? " on" : ""}`}
                  onClick={() => toggle(q, o.label)}
                >
                  <span>{o.label}</span>
                  {o.description && <small>{o.description}</small>}
                </button>
              ))}
            </div>
            <input
              value={typed[q.question] ?? ""}
              placeholder={t("chatView.askOther")}
              onChange={(e) => {
                const v = e.target.value;
                setTyped((x) => ({ ...x, [q.question]: v }));
                if (!q.multiSelect && v.trim()) setPicks((p) => ({ ...p, [q.question]: [] }));
              }}
            />
          </div>
        ))}
        <div className="chat-ask-btns">
          <button type="button" className="pixel-btn primary" disabled={busy || !ready} onClick={() => void run("allow", answers)}>{t("chatView.askSend")}</button>
          <button type="button" className="pixel-btn" disabled={busy} onClick={() => void run("dismiss")}>{t("chatView.dismiss")}</button>
        </div>
        <div className="chat-ask-note">{remote ? t("chatView.remoteAskNote") : t("chatView.askNote")}</div>
      </div>
    );
  }

  const s = permissionSummary(ask);
  return (
    <div className="chat-ask">
      <div className="chat-ask-title">{ask.sidechain ? "⤷ " : ""}{title}</div>
      {s.target && <div className="chat-ask-target">{s.target}</div>}
      <CopyableText className="chat-ask-body" text={s.body} onNotice={onNotice} />
      {s.note && <div className="chat-ask-target">{s.note}</div>}
      <input value={reason} placeholder={t("chatView.denyPh")} onChange={(e) => setReason(e.target.value)} />
      <div className="chat-ask-btns">
        <button type="button" className="pixel-btn primary" disabled={busy} onClick={() => void run("allow")}>{t("chatView.allow")}</button>
        {ask.canAlways && (
          <button
            type="button"
            className="pixel-btn"
            disabled={busy}
            onClick={() => {
              // 규칙이 남는 허용이라 한 번 더 눌러 확인한다.
              if (!armed) { setArmed(true); setTimeout(() => setArmed(false), 3000); return; }
              void run("always");
            }}
          >
            {armed ? t("chatView.again") : t("chatView.allowAlways")}
          </button>
        )}
        <button type="button" className="pixel-btn danger" disabled={busy} onClick={() => void run("deny", undefined, reason.trim() || undefined)}>{t("chatView.deny")}</button>
        <button type="button" className="pixel-btn" disabled={busy} onClick={() => void run("dismiss")}>{t("chatView.dismiss")}</button>
      </div>
      <div className="chat-ask-note">{remote ? t("chatView.remoteAskNote") : t("chatView.askNote")}</div>
    </div>
  );
}
