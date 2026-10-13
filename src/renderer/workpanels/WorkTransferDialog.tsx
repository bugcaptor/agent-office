import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { WorkStatus } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore, WORK_STATUS_ORDER } from "./workItemStore";
import {
  assigneeKey, guessAssignee, missingAssignees, MISSING_ASSIGNEE, parseWorkItemsFile, remapAssignees, serializeWorkItems, workItemsFileName,
  type ImportedWorkItem, type MissingAssignee,
} from "./workItemFile";
import { WorkDialog } from "./WorkDialog";

type Note = { text: string; error?: boolean };
type Pending = { items: ImportedWorkItem[]; skipped: number; missing: MissingAssignee[]; replace: Record<string, string> };

/** 업무 내보내기·가져오기 팝업. 다이얼로그 취소는 안내 없이 원래 상태로 돌아간다. */
export function WorkTransferDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation("workpanels");
  const items = useWorkItemStore((s) => s.items);
  const storeError = useWorkItemStore((s) => s.error);
  const agents = useAppStore((s) => s.agents);
  const [statuses, setStatuses] = useState<WorkStatus[]>(WORK_STATUS_ORDER);
  // 담당은 뺀 것만 기억해 처음과 새로 생긴 담당이 모두 고른 상태가 되게 한다.
  const [excluded, setExcluded] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<Note | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const byStatus = items.filter((item) => statuses.includes(item.status));
  const exportItems = byStatus.filter((item) => !excluded.includes(assigneeKey(item, agents)));
  // 담당 선택지는 업무가 있는 담당만 미배정·캐릭터·없는 캐릭터 순으로 보인다. 수는 고른 상태 안에서 센다.
  const keys = new Set(items.map((item) => assigneeKey(item, agents)));
  const assigneeOptions = ["", ...Object.keys(agents), MISSING_ASSIGNEE].filter((key) => keys.has(key)).map((key) => ({
    key,
    name: key === "" ? t("unassigned") : key === MISSING_ASSIGNEE ? t("missingAgent") : agents[key].name,
    count: byStatus.filter((item) => assigneeKey(item, agents) === key).length,
  }));
  const toggle = (status: WorkStatus, on: boolean) =>
    setStatuses((current) => WORK_STATUS_ORDER.filter((s) => s === status ? on : current.includes(s)));
  const toggleAssignee = (key: string, on: boolean) =>
    setExcluded((current) => on ? current.filter((k) => k !== key) : [...current, key]);
  const run = async (task: () => Promise<void>) => {
    setBusy(true); setNote(null); setPending(null);
    try { await task(); } finally { setBusy(false); }
  };
  const onExport = () => run(async () => {
    const names = Object.fromEntries(Object.values(agents).map((agent) => [agent.id, agent.name]));
    const { json, count } = serializeWorkItems(exportItems, statuses, undefined, names);
    try {
      const saved = await tauriApi.exportWorkItemsFile(workItemsFileName(), t("transfer.fileFilter"), json);
      if (saved) setNote({ text: t("transfer.exported", { count, path: saved }) });
    } catch (err) {
      console.warn("work-items: export failed", err);
      setNote({ text: t("transfer.exportFailed"), error: true });
    }
  });
  const onPick = () => run(async () => {
    let source: string | null;
    try { source = await tauriApi.importWorkItemsFile(t("transfer.fileFilter")); } catch (err) {
      console.warn("work-items: import read failed", err);
      setNote({ text: t("transfer.importFailed"), error: true });
      return;
    }
    if (source == null) return;
    const parsed = parseWorkItemsFile(source);
    if (!parsed.ok) { setNote({ text: t(`transfer.errors.${parsed.error}`), error: true }); return; }
    const missing = missingAssignees(parsed.items, agents);
    const list = Object.values(agents);
    setPending({ ...parsed, missing, replace: Object.fromEntries(missing.map((m) => [m.id, guessAssignee(m, list)])) });
  });
  const onConfirm = () => {
    if (!pending) return;
    // 확인하는 사이 캐릭터가 지워졌으면 그 대체는 미배정으로 둔다.
    const replace = Object.fromEntries(Object.entries(pending.replace).map(([id, to]) => [id, agents[to] ? to : ""]));
    const added = useWorkItemStore.getState().importFile(remapAssignees(pending.items, replace));
    setPending(null);
    if (!added) { setNote({ text: t("transfer.importBlocked"), error: true }); return; }
    void useWorkItemStore.getState().flush();
    setNote({ text: t("transfer.imported", { count: added }) });
  };
  return <WorkDialog title={t("transfer.title")} onClose={onClose}>
    <section className="work-transfer-section">
      <h3>{t("transfer.exportHeading")}</h3>
      <fieldset className="work-transfer-statuses">
        <legend>{t("transfer.statusesLegend")}</legend>
        {WORK_STATUS_ORDER.map((status) => <label key={status}>
          <input type="checkbox" checked={statuses.includes(status)} disabled={busy} onChange={(e) => toggle(status, e.target.checked)} />
          {t("transfer.statusOption", { status: t(`status.${status}`), count: items.filter((item) => item.status === status).length })}
        </label>)}
      </fieldset>
      {assigneeOptions.length > 0 && <fieldset className="work-transfer-statuses">
        <legend>{t("transfer.assigneesLegend")}</legend>
        {assigneeOptions.map(({ key, name, count }) => <label key={key}>
          <input type="checkbox" checked={!excluded.includes(key)} disabled={busy} onChange={(e) => toggleAssignee(key, e.target.checked)} />
          {t("transfer.assigneeOption", { name, count })}
        </label>)}
      </fieldset>}
      <p className="work-muted">{t("transfer.exportHint")}</p>
      <div className="work-dialog-actions">
        <span className="work-dialog-spacer" />
        <button className="pixel-btn primary" disabled={busy || !exportItems.length} onClick={() => void onExport()}>{t("transfer.export", { count: exportItems.length })}</button>
      </div>
    </section>
    <section className="work-transfer-section work-dialog-run">
      <h3>{t("transfer.importHeading")}</h3>
      <p className="work-muted">{t("transfer.importHint")}</p>
      {storeError && <p className="work-send-hint">{t("transfer.importBlocked")}</p>}
      {pending ? <>
        <p>{t("transfer.importPreview", { count: pending.items.length })}{pending.skipped > 0 && ` ${t("transfer.importSkipped", { count: pending.skipped })}`}</p>
        {pending.missing.length > 0 && <fieldset className="work-transfer-replace">
          <legend>{t("transfer.replaceLegend")}</legend>
          <p className="work-muted">{t("transfer.replaceHint")}</p>
          {pending.missing.map((m) => { const label = t("transfer.replaceOption", { name: m.name ?? m.id, count: m.count }); return <label key={m.id}>
            {label}
            <select aria-label={label} value={pending.replace[m.id] ?? ""} onChange={(e) => setPending({ ...pending, replace: { ...pending.replace, [m.id]: e.target.value } })}>
              <option value="">{t("unassigned")}</option>
              {Object.values(agents).map((agent) => <option key={agent.id} value={agent.id}>{agent.name}</option>)}
            </select>
          </label>; })}
        </fieldset>}
        <div className="work-dialog-actions">
          <button className="pixel-btn" onClick={() => setPending(null)}>{t("cancel")}</button>
          <span className="work-dialog-spacer" />
          <button className="pixel-btn primary" disabled={!!storeError} onClick={onConfirm}>{t("transfer.importConfirm")}</button>
        </div>
      </> : <div className="work-dialog-actions">
        <span className="work-dialog-spacer" />
        <button className="pixel-btn" disabled={busy || !!storeError} onClick={() => void onPick()}>{t("transfer.pick")}</button>
      </div>}
    </section>
    {note && <p className="work-transfer-note" role={note.error ? "alert" : "status"}>{note.text}</p>}
  </WorkDialog>;
}
