// src/renderer/files/FilesDialog.tsx
//
// 머신 간 파일 보내기 대화상자(`modal.kind === "files"`). 위는 보내기(대상 고르기 → 파일·폴더 고르기 → 메모·
// 세션 알림), 아래는 보낸·받은 기록(진행률·취소·이어 보내기·Finder에서 보기·지우기). 하단 바 📁, 터미널 탭
// 우클릭 메뉴(그 캐릭터에게), 원격 캐릭터 대화 창 머리(그 머신 캐릭터에게)에서 연다.
// `FileArrivals`는 방금 받은 것을 화면 아래 왼쪽에 잠깐 보인다(App에 늘 붙어 있다).
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { FileTarget, FileTransfer } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import {
  direction,
  dismissArrival,
  fileErrorKey,
  formatBytes,
  partyLabel,
  percent,
  refreshTargets,
  setFilesStatus,
  targetKey,
  useFilesStore,
} from "./filesStore";
import "./files.css";

function errorText(t: (k: string, o?: Record<string, unknown>) => string, code: string): string {
  const { key, detail } = fileErrorKey(code);
  return t(`errors.${key}`, { defaultValue: code, detail });
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function FilesDialog() {
  const { t } = useTranslation("files");
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const openModal = useAppStore((s) => s.openModal);
  const open = modal.kind === "files";
  useEscapeToClose(open, closeModal);
  const status = useFilesStore((s) => s.status);
  const transfers = useFilesStore((s) => s.transfers);
  const targets = useFilesStore((s) => s.targets);
  const [target, setTarget] = useState("");
  const [paths, setPaths] = useState<string[]>([]);
  const [note, setNote] = useState("");
  const [notify, setNotify] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 열려 있는 동안 대상 목록(붙은 머신·받기 여부)을 새로 받는다.
  useEffect(() => {
    if (!open) return;
    void tauriApi.filesStatus().then(setFilesStatus).catch(() => {});
    void refreshTargets();
    const id = setInterval(() => void refreshTargets(), 3000);
    return () => clearInterval(id);
  }, [open]);

  // 열 때 고른 대상(탭 메뉴·원격 대화 창)을 맞춘다.
  const preset = open ? modal.target : undefined;
  useEffect(() => {
    if (!open) return;
    setError(null);
    setPaths([]);
    setNote("");
    setNotify(false);
    setTarget("");
  }, [open]);
  useEffect(() => {
    if (!open || !preset) return;
    const found = targets.find((x) => {
      if (preset.remoteId) {
        const [device, agent] = preset.remoteId.split(":");
        return x.deviceId === device && x.agentId === agent;
      }
      if (preset.agentId) return x.local && x.agentId === preset.agentId;
      return x.machineId === preset.machineId && !x.agentId;
    });
    if (found) {
      setTarget(targetKey(found));
      if (found.agentId) setNotify(true);
    }
    // 목록이 늦게 와도 한 번 더 맞춘다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, preset?.remoteId, preset?.agentId, preset?.machineId, targets.length > 0]);

  const groups = useMemo(() => {
    const map = new Map<string, { name: string; local: boolean; items: FileTarget[] }>();
    for (const x of targets) {
      const g = map.get(x.machineId) ?? { name: x.machineName, local: x.local, items: [] };
      g.items.push(x);
      map.set(x.machineId, g);
    }
    return [...map.values()];
  }, [targets]);

  if (!open) return null;

  const chosen = targets.find((x) => targetKey(x) === target);
  const unavailable = (x: FileTarget) => (!x.online ? t("offline") : !x.receive ? t("receiveOff") : null);
  const canSend = !!chosen && !unavailable(chosen) && paths.length > 0 && !busy;

  const pick = async (folders: boolean) => {
    try {
      const picked = await tauriApi.filesPick(folders);
      if (picked.length) setPaths((old) => [...old, ...picked.filter((p) => !old.includes(p))]);
    } catch (err) {
      setError(String(err));
    }
  };

  const send = async () => {
    if (!chosen) return;
    setBusy(true);
    setError(null);
    try {
      await tauriApi.filesSend({
        machineId: chosen.machineId,
        agentId: chosen.agentId,
        paths,
        note: note.trim() || undefined,
        notify: notify || !!note.trim(),
        fromAgent: open ? modal.fromAgent : undefined,
      });
      setPaths([]);
      setNote("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const sorted = [...transfers].sort((a, b) => b.createdAt - a.createdAt);

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel files-dialog" role="dialog" aria-label={t("title")}>
        <div className="files-head">
          <h2 className="pixel-title">{t("title")}</h2>
          <button className="pixel-btn" onClick={() => void tauriApi.filesReveal().catch(() => {})}>
            {t("openFolder")}
          </button>
        </div>
        {status && (
          <p className="files-receive-line">
            {status.config.receiveEnabled ? t("receiveOnLine", { dir: status.receiveDir }) : t("receiveOffLine")}{" "}
            <button className="files-link" onClick={() => openModal({ kind: "settings" })}>
              {t("changeInSettings")}
            </button>
          </p>
        )}

        <section className="files-send">
          <label className="files-field">
            <span>{t("to")}</span>
            <select value={target} onChange={(e) => setTarget(e.target.value)} disabled={busy}>
              <option value="">{t("chooseTarget")}</option>
              {groups.map((g) => (
                <optgroup key={g.name + g.local} label={g.local ? t("thisMachine", { name: g.name }) : g.name}>
                  {g.items.map((x) => {
                    const why = unavailable(x);
                    const label = x.agentName ? `${x.agentName} · ${x.machineName}` : t("machineItself", { name: x.machineName });
                    return (
                      <option key={targetKey(x)} value={targetKey(x)} disabled={!!why}>
                        {why ? `${label} (${why})` : label}
                      </option>
                    );
                  })}
                </optgroup>
              ))}
            </select>
          </label>
          {!status?.linked && <small className="files-hint">{t("notLinked")}</small>}
          <div className="files-paths">
            {paths.length === 0 ? (
              <small className="files-hint">{t("noPaths")}</small>
            ) : (
              <ul>
                {paths.map((p) => (
                  <li key={p}>
                    <span title={p}>{baseName(p)}</span>
                    <small>{p}</small>
                    <button className="files-x" aria-label={t("remove")} onClick={() => setPaths((old) => old.filter((x) => x !== p))}>
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <div className="files-row">
              <button className="pixel-btn" disabled={busy} onClick={() => void pick(false)}>{t("pickFiles")}</button>
              <button className="pixel-btn" disabled={busy} onClick={() => void pick(true)}>{t("pickFolders")}</button>
            </div>
          </div>
          {chosen?.agentId && (
            <>
              <label className="files-field">
                <span>{t("note")}</span>
                <textarea rows={2} value={note} maxLength={4000} placeholder={t("notePlaceholder")} onChange={(e) => setNote(e.target.value)} />
              </label>
              <label className="files-check">
                <input type="checkbox" checked={notify || !!note.trim()} disabled={!!note.trim()} onChange={(e) => setNotify(e.target.checked)} />
                <span>{t("notify", { name: chosen.agentName })}</span>
              </label>
            </>
          )}
          {error && <div className="files-error" role="alert">{errorText(t, error)}</div>}
          <div className="dialog-actions">
            <button className="pixel-btn primary" disabled={!canSend} onClick={() => void send()}>
              {busy ? t("sending") : t("send")}
            </button>
            <button className="pixel-btn" onClick={closeModal}>{t("close")}</button>
          </div>
        </section>

        <section className="files-history">
          <h3>{t("history")}</h3>
          {sorted.length === 0 ? <small className="files-hint">{t("empty")}</small> : (
            <ul>
              {sorted.map((x) => <TransferRow key={x.id} transfer={x} machineId={status?.machineId} />)}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

function TransferRow({ transfer: x, machineId }: { transfer: FileTransfer; machineId?: string }) {
  const { t } = useTranslation("files");
  const [error, setError] = useState<string | null>(null);
  const dir = direction(x, machineId);
  const act = (call: () => Promise<void>) => {
    setError(null);
    call().catch((err) => setError(err instanceof Error ? err.message : String(err)));
  };
  const active = x.state === "preparing" || x.state === "sending";
  const here = dir === "received" || dir === "local";
  const icon = dir === "sent" ? "↑" : dir === "received" ? "↓" : dir === "local" ? "↔" : "⇄";
  const who = dir === "received" ? t("fromWho", { who: partyLabel(x.from) }) : dir === "relay" ? t("relayWho", { from: partyLabel(x.from), to: partyLabel(x.to) }) : t("toWho", { who: partyLabel(x.to) });
  return (
    <li className={`files-transfer files-${x.state}`}>
      <span className="files-dir" aria-hidden>{icon}</span>
      <span className="files-main">
        <span className="files-items" title={x.items.join(", ")}>{x.items.join(", ")}</span>
        <small>
          {who} · {t("count", { count: x.files.length })} · {formatBytes(x.totalBytes)} · {new Date(x.createdAt).toLocaleString()}
        </small>
        {x.state === "sending" && (
          <span className="files-bar" aria-label={t("progress", { pct: percent(x) })}>
            <span style={{ width: `${percent(x)}%` }} />
          </span>
        )}
        <small className="files-state">
          {t(`state.${x.state}`, { pct: percent(x) })}
          {x.state === "failed" && x.error ? ` — ${errorText(t, x.error)}` : ""}
        </small>
        {x.note && <small className="files-note">“{x.note}”</small>}
        {error && <small className="files-error">{errorText(t, error)}</small>}
      </span>
      <span className="files-actions">
        {active && <button className="pixel-btn" onClick={() => act(() => tauriApi.filesCancel(x.id))}>{t("cancel")}</button>}
        {x.state === "failed" && (dir === "sent" || dir === "local" || dir === "relay") && (
          <button className="pixel-btn" onClick={() => act(() => tauriApi.filesResume(x.id))}>{t("resume")}</button>
        )}
        {x.state === "done" && here && x.saved?.[0] && (
          <button className="pixel-btn" onClick={() => act(() => tauriApi.filesReveal(x.saved![0]))}>{t("reveal")}</button>
        )}
        {!active && <button className="files-x" aria-label={t("forget")} title={t("forget")} onClick={() => act(() => tauriApi.filesForget(x.id))}>×</button>}
      </span>
    </li>
  );
}

/** 방금 다 받은 것(화면 아래 왼쪽, 12초). */
export function FileArrivals() {
  const { t } = useTranslation("files");
  const arrivals = useFilesStore((s) => s.arrivals);
  if (!arrivals.length) return null;
  return (
    <div className="files-arrivals" aria-live="polite">
      {arrivals.map((x) => (
        <div key={x.id} className="pixel-panel files-arrival">
          <span>
            {t("arrived", { who: partyLabel(x.from), items: x.items.join(", "), to: x.to.agentName ?? x.to.machineName })}
          </span>
          {x.saved?.[0] && (
            <button className="pixel-btn" onClick={() => void tauriApi.filesReveal(x.saved![0]).catch(() => {})}>{t("reveal")}</button>
          )}
          <button className="files-x" aria-label={t("close")} onClick={() => dismissArrival(x.id)}>×</button>
        </div>
      ))}
    </div>
  );
}
