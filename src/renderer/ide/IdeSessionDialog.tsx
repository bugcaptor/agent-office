import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { flushPersistence } from "../store/persist";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { currentLocale } from "../i18n";
import type { IdeSessionCandidate, IdeSessionConnectResult } from "@shared/types";

type Replacement = Extract<IdeSessionConnectResult, { replacement: unknown }>["replacement"];
type PendingReplacement = { agentId: string; candidate: IdeSessionCandidate; replacement: Replacement };

function candidateKey(candidate: IdeSessionCandidate): string {
  return `${candidate.provider}:${candidate.sourceSessionId}:${candidate.file}`;
}

export function IdeSessionDialog() {
  const { t } = useTranslation("profile");
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const appSettings = useAppStore((s) => s.appSettings);
  const agents = useAppStore((s) => s.agents);
  const agentOrder = useAppStore((s) => s.agentOrder);
  const openTerminal = useAppStore((s) => s.openTerminal);
  const setSessionState = useAppStore((s) => s.setSessionState);
  const openModal = useAppStore((s) => s.openModal);
  const [provider, setProvider] = useState<"all" | IdeSessionCandidate["provider"]>("all");
  const [items, setItems] = useState<IdeSessionCandidate[]>([]);
  const [selected, setSelected] = useState<IdeSessionCandidate | null>(null);
  const [agentId, setAgentId] = useState("");
  const [loading, setLoading] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState("");
  const [pending, setPending] = useState<PendingReplacement | null>(null);
  const connectingRef = useRef(false);
  const refreshGeneration = useRef(0);
  const open = modal.kind === "ide-session";
  const cancel = () => pending ? setPending(null) : closeModal();
  useEscapeToClose(open && !connecting, cancel);

  const ideConnectionEnabled = appSettings.ideConnectionEnabled;
  const observationAvailable = ideConnectionEnabled && appSettings.observerEnabled;
  const refresh = async () => {
    if (!useAppStore.getState().appSettings.ideConnectionEnabled || !useAppStore.getState().appSettings.observerEnabled) return;
    const generation = ++refreshGeneration.current;
    setLoading(true);
    setError("");
    try {
      const result = await tauriApi.listIdeSessions(provider === "all" ? undefined : { provider });
      if (generation !== refreshGeneration.current) return;
      setItems(result);
      setSelected((old) => result.find((entry) => old && candidateKey(entry) === candidateKey(old)) ?? null);
    } catch (err) {
      if (generation !== refreshGeneration.current) return;
      console.warn("list IDE sessions failed", err);
      setError(t("ide.loadFailed"));
    } finally {
      if (generation === refreshGeneration.current) setLoading(false);
    }
  };

  useEffect(() => {
    if (open && observationAvailable) void refresh();
  // provider intentionally refreshes the list while the dialog is open.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, provider, observationAvailable]);

  useEffect(() => {
    if (observationAvailable) return;
    // In-flight calls are made stale and their result must not re-enable UI.
    refreshGeneration.current += 1;
    setLoading(false);
    setConnecting(false);
    setItems([]);
    setSelected(null);
    setPending(null);
  }, [observationAvailable]);

  const eligibleAgents = useMemo(() => agentOrder.map((id) => agents[id]).filter(Boolean), [agentOrder, agents]);

  useEffect(() => {
    if (!open) setPending(null);
  }, [open]);

  useEffect(() => {
    if (!eligibleAgents.some((agent) => agent.id === agentId)) setAgentId(eligibleAgents[0]?.id ?? "");
  }, [agentId, eligibleAgents]);

  const connect = async (confirmed?: PendingReplacement) => {
    const state = useAppStore.getState();
    const target = confirmed?.candidate ?? selected;
    const targetId = confirmed?.agentId ?? agentId;
    if (connectingRef.current || !state.appSettings.ideConnectionEnabled || !state.appSettings.observerEnabled || !target || !state.agents[targetId]) return;
    connectingRef.current = true;
    setConnecting(true);
    setError("");
    try {
      // Save profile creation/edits first. Selecting or declining replacement
      // must never mutate the profile or dispose its terminal.
      await flushPersistence();
      const result = await tauriApi.connectIdeSession({
        agentId: targetId,
        provider: target.provider,
        file: target.file,
        sourceSessionId: target.sourceSessionId,
        ...(confirmed ? { replaceSessionId: confirmed.replacement.sessionId } : {}),
      });
      const current = useAppStore.getState();
      if (!current.appSettings.ideConnectionEnabled || !current.appSettings.observerEnabled) return;
      if ("replacement" in result) {
        setPending({ agentId: targetId, candidate: target, replacement: result.replacement });
        return;
      }
      setPending(null);
      // Revive the character without clockInAgent(), which would start a PTY.
      current.clockIn(targetId);
      current.updateAgent(targetId, { cwd: target.cwd });
      current.resetAutomationState(targetId);
      setSessionState({ agentId: targetId, status: "running", external: true });
      current.noteUsageSession(targetId, result.sessionId);
      // Persist the connected folder before persona setup reads the profile.
      await flushPersistence().catch((err) => console.warn("save connected IDE profile failed", err));
      openTerminal(targetId);
      closeModal();
    } catch (err) {
      console.warn("connect IDE session failed", err);
      setPending(null);
      const reason = String(err);
      setError(t(reason.includes("observed-replacement-changed") ? "ide.replacementChanged"
        : /observed-replacement-(?:tmux-)?kill-failed/.test(reason) ? "ide.replacementKillFailed"
        : "ide.connectFailed"));
    } finally {
      connectingRef.current = false;
      setConnecting(false);
    }
  };

  if (!open) return null;
  return <div className="modal-backdrop" onMouseDown={(e) => {
    if (!connecting && e.button === 0 && e.target === e.currentTarget) cancel();
  }}>
    {pending ? <div className="pixel-panel ide-session-dialog" role="alertdialog" aria-modal="true" aria-labelledby="ide-replace-title" aria-describedby="ide-replace-body">
      <h2 id="ide-replace-title" className="pixel-title">{t("ide.replaceTitle")}</h2>
      <p id="ide-replace-body">{t(pending.replacement.kind === "pty" ? "ide.replaceTerminalBody" : "ide.replaceExternalBody", { name: agents[pending.agentId]?.name, cwd: pending.candidate.cwd })}</p>
      <div className="dialog-actions">
        <button type="button" className="pixel-btn primary" disabled={!observationAvailable || connecting} onClick={() => void connect(pending)}>{t(pending.replacement.kind === "pty" ? "ide.replaceTerminalConfirm" : "ide.replaceExternalConfirm")}</button>
        <button type="button" className="pixel-btn" autoFocus disabled={connecting} onClick={() => setPending(null)}>{t("dialog.cancel")}</button>
      </div>
    </div> : <div className="pixel-panel ide-session-dialog" role="dialog" aria-modal="true" aria-label={t("ide.title")}>
      <header className="profile-dialog-header">
        <h2 className="pixel-title">{t("ide.title")}</h2>
        <p className="profile-dialog-sub">{t("ide.description")}</p>
        <p className="ide-session-note">{t("ide.availabilityNote")}</p>
        <p className="ide-session-note">{t("ide.historyNote")}</p>
      </header>
      {!ideConnectionEnabled ? <div role="alert" className="ide-session-note">{t("ideConnection.disabled")}</div> : !appSettings.observerEnabled ? <div role="alert" className="ide-session-note">{t("ide.observerDisabled")}</div> : <>
        <div className="ide-session-toolbar">
          <label>{t("ide.provider")}<select value={provider} disabled={connecting} onChange={(e) => setProvider(e.target.value as typeof provider)}>
            <option value="all">{t("ide.providerAll")}</option>
            {(["codex", "claude", "kilo"] as const).map((value) => <option key={value} value={value}>{t(`ide.providers.${value}`)}</option>)}
          </select></label>
          <button type="button" className="pixel-btn" onClick={() => void refresh()} disabled={loading || connecting}>{t("ide.refresh")}</button>
        </div>
        {(provider === "kilo" || items.some((item) => item.source === "kilo-shared")) && <p className="ide-session-note">{t("ide.kiloSourceNote")}</p>}
        {loading && <p>{t("ide.loading")}</p>}
        {!loading && items.length === 0 && <p className="ide-session-note">{t("ide.empty")}</p>}
        <div className="ide-session-list" role="group" aria-label={t("ide.candidates")}>
          {items.map((item) => <button key={candidateKey(item)} type="button" aria-pressed={Boolean(selected && candidateKey(selected) === candidateKey(item))} disabled={connecting} className={`ide-session-candidate ${selected && candidateKey(selected) === candidateKey(item) ? "selected" : ""}`} onClick={() => setSelected(item)} title={item.file}>
            <strong>{t(`ide.providers.${item.provider}`)}</strong><span>{item.cwd}</span><small>{t("ide.updated", { value: new Intl.DateTimeFormat(currentLocale(), { dateStyle: "short", timeStyle: "short" }).format(item.updatedAt) })} · {t("ide.sessionId", { value: item.sourceSessionId })}</small>
          </button>)}
        </div>
        {selected && <p className="ide-session-note">{t("ide.characterFolderNote", { cwd: selected.cwd })}</p>}
        {selected && <label className="ide-session-agent">{t("ide.agent")}<select value={agentId} disabled={connecting} onChange={(e) => setAgentId(e.target.value)}>
          {eligibleAgents.map((agent) => <option key={agent.id} value={agent.id}>{agent.name} · {agent.role}</option>)}
        </select></label>}
        {selected && eligibleAgents.length === 0 && <div className="ide-session-note"><p>{t("ide.noAgent")}</p><button type="button" className="pixel-btn" disabled={connecting} onClick={() => openModal({ kind: "profile-create", initialCwd: selected.cwd, returnToIdeSession: true })}>{t("ide.createAgent")}</button></div>}
      </>}
      {error && <p role="alert">{error}</p>}
      <div className="dialog-actions">
        <button type="button" className="pixel-btn primary" disabled={!observationAvailable || !selected || !agentId || connecting} onClick={() => void connect()}>{t("ide.connect")}</button>
        <button type="button" className="pixel-btn" disabled={connecting} onClick={closeModal}>{t("dialog.cancel")}</button>
      </div>
    </div>}
  </div>;
}
