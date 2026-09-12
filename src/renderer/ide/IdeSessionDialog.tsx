import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { flushPersistence } from "../store/persist";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { cwdEquivalent } from "../labels/labelText";
import { currentLocale } from "../i18n";
import type { IdeSessionCandidate } from "@shared/types";

function sameCwd(a: string | undefined, b: string): boolean {
  return Boolean(a) && cwdEquivalent(a!, b);
}

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
  const sessions = useAppStore((s) => s.sessions);
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
  const refreshGeneration = useRef(0);
  const open = modal.kind === "ide-session";
  useEscapeToClose(open && !connecting, closeModal);

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
  }, [observationAvailable]);

  const eligibleAgents = useMemo(() => {
    if (!selected) return [];
    return agentOrder
      .map((id) => agents[id])
      .filter((agent) => agent && !agent.clockedOut && sameCwd(agent.cwd, selected.cwd))
      .filter((agent) => {
        const state = sessions[agent.id]?.status;
        return state === "idle" || state === "exited";
      });
  }, [agentOrder, agents, selected, sessions]);

  useEffect(() => {
    if (!eligibleAgents.some((agent) => agent.id === agentId)) setAgentId(eligibleAgents[0]?.id ?? "");
  }, [agentId, eligibleAgents]);

  const connect = async () => {
    if (!useAppStore.getState().appSettings.ideConnectionEnabled || !useAppStore.getState().appSettings.observerEnabled || !selected || !agentId) return;
    setConnecting(true);
    setError("");
    try {
      // The backend validates the saved profile cwd. Do not let the normal
      // 500ms profile save debounce race this connection.
      await flushPersistence();
      await tauriApi.connectIdeSession({
        agentId,
        provider: selected.provider,
        file: selected.file,
        sourceSessionId: selected.sourceSessionId,
      });
      if (!useAppStore.getState().appSettings.ideConnectionEnabled || !useAppStore.getState().appSettings.observerEnabled) return;
      setSessionState({ agentId, status: "running", external: true });
      // This opens the connected-session panel directly. Calling ensureSession
      // here would accidentally create a new PTY for the character.
      openTerminal(agentId);
      closeModal();
    } catch (err) {
      console.warn("connect IDE session failed", err);
      setError(t("ide.connectFailed"));
    } finally {
      setConnecting(false);
    }
  };

  if (!open) return null;
  return <div className="modal-backdrop" onMouseDown={(e) => {
    if (!connecting && e.button === 0 && e.target === e.currentTarget) closeModal();
  }}>
    <div className="pixel-panel ide-session-dialog" role="dialog" aria-modal="true" aria-label={t("ide.title")}>
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
    </div>
  </div>;
}
