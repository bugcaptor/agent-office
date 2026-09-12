import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { tauriApi } from "../ipc/tauriApi";
import { flushPersistence } from "../store/persist";
import { useAppStore } from "../store/appStore";
import { isRemoteWindow } from "../shared/remoteWindow";
import type { IdePersonaContext } from "@shared/types";

type SetupState = "idle" | "ready" | "confirmed";

function errorKey(error: unknown): string {
  const code = String(error);
  if (code.includes("style-conflict")) return "persona.conflict";
  if (/stale|not-connected/.test(code)) return "persona.stale";
  if (/path-|style-(dir|read|create|write)|home-unavailable/.test(code)) return "persona.pathError";
  return "persona.error";
}

/** A profile or logical connection change discards every local acknowledgement. */
export function IdePersonaPanel({ agentId }: { agentId: string }) {
  const agent = useAppStore((s) => s.agents[agentId]);
  const sessionId = useAppStore((s) => s.sessionUsage[agentId]?.sessionId);
  if (isRemoteWindow() || !agent) return null;
  const prompt = agent.personalityPrompt ?? "";
  return <PersonaSetup key={JSON.stringify([agentId, sessionId, agent.name, agent.cwd, prompt])}
    agentId={agentId} prompt={prompt} />;
}

function PersonaSetup({ agentId, prompt }: { agentId: string; prompt: string }) {
  const { t } = useTranslation("terminal");
  const [context, setContext] = useState<IdePersonaContext | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [state, setState] = useState<SetupState>("idle");
  const [open, setOpen] = useState(true);
  const [revision, setRevision] = useState(0);
  const generation = useRef(0);
  const styleMessage = t("persona.codexMessage", { prompt, interpolation: { escapeValue: false } });
  const hasPrompt = Boolean(prompt.trim());

  useEffect(() => {
    const request = ++generation.current;
    setLoading(true); setError(""); setContext(null); setState("idle"); setBusy(false);
    void Promise.resolve().then(flushPersistence).then(() => {
      if (request !== generation.current) return null;
      return tauriApi.getIdePersona(agentId);
    }).then((value) => {
      if (request !== generation.current) return;
      if (value && value.personalityPrompt !== prompt) throw new Error("ide-persona-prompt-stale");
      setContext(value);
      if (value?.style?.exists && hasPrompt) setState("ready");
    }).catch((err) => {
      console.warn("get IDE persona failed", err);
      if (request === generation.current) setError(errorKey(err));
    }).finally(() => { if (request === generation.current) setLoading(false); });
    return () => { generation.current++; };
  }, [agentId, prompt, hasPrompt, revision]);

  // Recheck identity before copying or accepting a user's acknowledgement. This
  // checks our connection/artifact, never the provider's selected setting or reply.
  const checkCurrent = async () => {
    const current = await tauriApi.getIdePersona(agentId);
    if (!current || current.sessionId !== context?.sessionId || current.personalityPrompt !== prompt
      || current.provider !== context.provider || current.cwd !== context.cwd
      || current.sourceSessionId !== context.sourceSessionId || current.style?.path !== context.style?.path
      || current.style?.content !== context.style?.content) throw new Error("ide-persona-session-stale");
    return current;
  };

  const perform = async (action: "prepare" | "copy" | "confirm") => {
    if (!context || !hasPrompt || busy) return;
    const request = ++generation.current;
    setBusy(true); setError("");
    try {
      if (action === "prepare") {
        await flushPersistence();
        if (request !== generation.current) return;
        const next = await tauriApi.prepareIdePersona({ agentId, sessionId: context.sessionId, personalityPrompt: prompt });
        if (request !== generation.current) return;
        if (next.sessionId !== context.sessionId || next.personalityPrompt !== prompt || !next.style?.exists)
          throw new Error("ide-persona-session-stale");
        setContext(next); setState("ready");
      } else {
        // Start clipboard writing in the click's user-activation scope (WebKit
        // may drop that activation after an IPC await). Neither operation sends
        // anything to the IDE; readiness still requires the identity check.
        const current = action === "copy"
          ? (await Promise.all([navigator.clipboard.writeText(styleMessage), checkCurrent()]))[1]
          : await checkCurrent();
        if (request !== generation.current) return;
        if (action === "copy") {
          setState("ready");
        } else {
          if (current.provider === "claude" && !current.style?.exists) throw new Error("ide-persona-session-stale");
          setState("confirmed");
        }
      }
    } catch (err) {
      console.warn("IDE persona step failed", err);
      if (request === generation.current) { setError(errorKey(err)); setState("idle"); }
    } finally { if (request === generation.current) setBusy(false); }
  };

  const retry = <button type="button" className="pixel-btn" disabled={busy} onClick={() => setRevision((v) => v + 1)}>{t("persona.retry")}</button>;
  if (loading) return <p className="ide-persona-loading">{t("persona.loading")}</p>;
  if (!context) return error ? <div className="ide-persona-panel"><p role="alert">{t(error)}</p>{retry}</div> : null;
  const claude = context.provider === "claude";
  return <section className="ide-persona-panel" aria-label={t("persona.title")}>
    <button type="button" className="ide-persona-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>{t("persona.title")}</button>
    <p className="ide-persona-note">{t("persona.detachNote")}</p>
    {open && <div className="ide-persona-body">
      <p className="ide-persona-context">{t("persona.context", { provider: claude ? "Claude" : "Codex", id: context.sourceSessionId })}<br />{t("persona.workdir", { cwd: context.cwd })}</p>
      {!hasPrompt ? <><p>{t("persona.emptyPrompt")}</p><button type="button" className="pixel-btn" onClick={() => useAppStore.getState().openModal({ kind: "profile-edit", agentId })}>{t("persona.editProfile")}</button></> : <>
        <p className="ide-persona-status" role="status">{state === "confirmed" ? t("persona.confirmed") : state === "ready" ? t(claude ? "persona.prepareDone" : "persona.copied") : t("persona.unverified")}</p>
        {claude ? <>
          <p>{t("persona.claudeIntro")}</p>
          <p>{t("persona.claudeScope")}</p>
          <pre className="ide-persona-preview">{context.style?.content}</pre>
          <code className="ide-persona-path">{context.style?.path}</code>
          <button type="button" className="pixel-btn" disabled={busy} onClick={() => void perform("prepare")}>{busy ? t("persona.preparing") : context.style?.exists ? t("persona.styleReady") : t("persona.createStyle")}</button>
          <p>{t("persona.claudeSteps", { name: context.style?.name })}</p>
          <p>{t("persona.claudeTrouble")}</p>
        </> : <>
          <p>{t("persona.codexIntro")}</p>
          <pre className="ide-persona-preview">{styleMessage}</pre>
          <button type="button" className="pixel-btn" disabled={busy} onClick={() => void perform("copy")}>{t("persona.copy")}</button>
          <p>{t("persona.codexSteps")}</p>
        </>}
        {error && <div><p role="alert">{t(error)}</p>{retry}</div>}
        <button type="button" className="pixel-btn" disabled={busy || state !== "ready"} onClick={() => void perform("confirm")}>{t("persona.confirm")}</button>
        <p className="ide-persona-note">{t("persona.verificationNote")}</p>
        <p>{t(claude ? "persona.claudeRecovery" : "persona.codexRecovery")}</p>
      </>}
    </div>}
  </section>;
}
