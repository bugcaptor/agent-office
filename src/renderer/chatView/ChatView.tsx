// src/renderer/chatView/ChatView.tsx
//
// 터미널 탭의 "대화 보기". xterm은 아래에 그대로 붙어 있고(keep-alive), 이 판이 그 위를 덮는다.
// 내용은 CLI 전사(JSONL)다 — 사람 말·에이전트 말(마크다운)·도구 호출과 결과(접힘)·이미지.
// 권한 요청·질문은 카드로 뜨고 여기서 답할 수 있다. 터미널에도 같은 대화상자가 떠 있어
// 어느 보기에서든 먼저 답한 쪽이 반영된다. 아래 입력칸은 CLI가 입력을 기다릴 때 같은 세션에 잇는다.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AskAction, ChatLogItem, ChatLogPage, PendingAsk } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { renderMarkdown } from "../markdown/renderMarkdown";
import {
  askQuestions,
  groupChatItems,
  joinAnswers,
  mergeChatPage,
  permissionSummary,
  type ChatBlock,
} from "./chatViewModel";
import "./chatView.css";

const POLL_MS = 1500;
/** 아래 끝에서 이만큼 안쪽이면 "맨 아래를 보고 있다"로 보고 새 항목에 따라 내린다. */
const STICK_PX = 80;

/** 이미지 data URL 캐시(탭을 오가도 다시 받지 않는다). null은 비워진 이미지. */
const imageCache = new Map<string, string | null>();

function errorCode(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function ChatView({ agentId, active, onShowTerminal }: { agentId: string; active: boolean; onShowTerminal: () => void }) {
  const { t } = useTranslation("terminal");
  const [page, setPage] = useState<ChatLogPage | null>(null);
  const [asks, setAsks] = useState<PendingAsk[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [zoom, setZoom] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const stick = useRef(true);
  const pageRef = useRef<ChatLogPage | null>(null);
  pageRef.current = page;

  const errText = (code: string) => t(`chatView.err.${code}`, { defaultValue: code });

  // 보이는 동안만 묻는다. 탭을 떠나면 멈추고, 돌아오면 이어 받는다.
  useEffect(() => {
    if (!active) return;
    let stop = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      try {
        const prev = pageRef.current;
        const [next, pending] = await Promise.all([
          tauriApi.chatViewLog(agentId, prev?.last, prev?.epoch),
          tauriApi.chatViewAsks(agentId),
        ]);
        if (stop) return;
        setPage((p) => mergeChatPage(p, next));
        setAsks((a) => (JSON.stringify(a) === JSON.stringify(pending) ? a : pending));
      } catch {
        // 앱이 바쁘거나 세션이 바뀌는 중 — 다음 틱에 다시 묻는다.
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

  const blocks = useMemo(() => groupChatItems(page?.items ?? []), [page]);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [blocks, asks]);

  const send = async () => {
    const text = draft.trim();
    if (!text || sending) return;
    setSending(true);
    setNotice(null);
    try {
      await tauriApi.chatViewSend(agentId, text);
      setDraft("");
      stick.current = true;
    } catch (err) {
      setNotice(errText(errorCode(err)));
    } finally {
      setSending(false);
    }
  };

  const answer = async (ask: PendingAsk, action: AskAction, answers?: Record<string, string>, message?: string) => {
    try {
      await tauriApi.chatViewAnswer(ask.id, action, answers, message);
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
        <span className="chat-view-bar-title">{t("chatView.title")}</span>
        <span className="chat-view-bar-hint">{t("chatView.hint")}</span>
        <button type="button" className="pixel-btn" onClick={onShowTerminal}>{t("chatView.showTerminal")}</button>
      </div>
      <div
        className="chat-view-scroll"
        ref={scrollRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX;
        }}
      >
        {!page ? (
          <div className="chat-view-empty">…</div>
        ) : !page.attached ? (
          <div className="chat-view-empty">{t("chatView.noLog")}</div>
        ) : !blocks.length ? (
          <div className="chat-view-empty">{t("chatView.noItems")}</div>
        ) : (
          blocks.map((b) => <Block key={b.key} block={b} agentId={agentId} onZoom={setZoom} />)
        )}
        {asks.map((a) => <AskCard key={a.id} ask={a} onAnswer={answer} />)}
      </div>
      {notice && <div className="chat-view-notice" role="status">{notice}</div>}
      <div className="chat-view-input">
        <textarea
          ref={inputRef}
          value={draft}
          placeholder={t("chatView.inputPh")}
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
          {t("chatView.send")}
        </button>
      </div>
      {zoom && (
        <div className="chat-view-zoom" role="dialog" onClick={() => setZoom(null)}>
          <img src={zoom} alt="" />
        </div>
      )}
    </div>
  );
}

function Block({ block, agentId, onZoom }: { block: ChatBlock; agentId: string; onZoom: (src: string) => void }) {
  return (
    <div className={`chat-block${block.side ? " side" : ""}`}>
      <Item item={block.head} agentId={agentId} onZoom={onZoom} />
      {block.children.map((c) => <Item key={c.seq} item={c} agentId={agentId} onZoom={onZoom} nested />)}
    </div>
  );
}

function Item({ item, agentId, onZoom, nested = false }: { item: ChatLogItem; agentId: string; onZoom: (src: string) => void; nested?: boolean }) {
  const { t } = useTranslation("terminal");
  if (item.kind === "image") return <ChatImage item={item} agentId={agentId} onZoom={onZoom} />;
  if (item.kind === "text") {
    if (item.role === "assistant") {
      return (
        <div
          className="chat-msg assistant markdown-body"
          // 링크는 웹뷰 안에서 따라가지 않는다.
          onClick={(e) => { if ((e.target as HTMLElement).closest("a")) e.preventDefault(); }}
          dangerouslySetInnerHTML={{ __html: renderMarkdown(item.text) }}
        />
      );
    }
    return <div className="chat-msg user">{item.text}</div>;
  }
  if (item.kind === "tool_use") {
    const first = item.text.split("\n").find((l) => l.trim()) ?? "";
    if (!item.toolName) return <div className="chat-tool plain">⤷ {first}</div>;
    return (
      <details className="chat-tool">
        <summary>🔧 {item.toolName}: {first}</summary>
        <pre>{item.text}</pre>
      </details>
    );
  }
  const lines = item.text.split("\n").length;
  const first = item.text.split("\n").find((l) => l.trim()) ?? "";
  return (
    <details className={`chat-tool${item.isError ? " err" : ""}${nested ? " nested" : ""}`}>
      <summary>↳ {t("chatView.result", { count: lines })} · {first.slice(0, 80)}</summary>
      <pre>{item.text}</pre>
    </details>
  );
}

function ChatImage({ item, agentId, onZoom }: { item: ChatLogItem; agentId: string; onZoom: (src: string) => void }) {
  const { t } = useTranslation("terminal");
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
  if (src === null) return <div className="chat-tool plain">{t("chatView.imageGone")}</div>;
  if (src === undefined) return <div className="chat-image loading" />;
  return (
    <button type="button" className={`chat-image ${item.role}`} onClick={() => onZoom(src)} title={t("chatView.zoom")}>
      <img src={src} alt="" />
    </button>
  );
}

function AskCard({ ask, onAnswer }: { ask: PendingAsk; onAnswer: (ask: PendingAsk, action: AskAction, answers?: Record<string, string>, message?: string) => Promise<void> }) {
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
        <div className="chat-ask-note">{t("chatView.askNote")}</div>
      </div>
    );
  }

  const s = permissionSummary(ask);
  return (
    <div className="chat-ask">
      <div className="chat-ask-title">{ask.sidechain ? "⤷ " : ""}{title}</div>
      {s.target && <div className="chat-ask-target">{s.target}</div>}
      <pre className="chat-ask-body">{s.body}</pre>
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
      <div className="chat-ask-note">{t("chatView.askNote")}</div>
    </div>
  );
}
