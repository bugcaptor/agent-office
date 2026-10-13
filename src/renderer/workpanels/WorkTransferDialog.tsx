import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { WorkStatus } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useWorkItemStore, WORK_STATUS_ORDER } from "./workItemStore";
import { parseWorkItemsFile, serializeWorkItems, workItemsFileName, type ImportedWorkItem } from "./workItemFile";
import { WorkDialog } from "./WorkDialog";

type Note = { text: string; error?: boolean };

/** 업무 내보내기·가져오기 팝업. 다이얼로그 취소는 안내 없이 원래 상태로 돌아간다. */
export function WorkTransferDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation("workpanels");
  const items = useWorkItemStore((s) => s.items);
  const storeError = useWorkItemStore((s) => s.error);
  const [statuses, setStatuses] = useState<WorkStatus[]>(WORK_STATUS_ORDER);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<Note | null>(null);
  const [pending, setPending] = useState<{ items: ImportedWorkItem[]; skipped: number } | null>(null);
  const exportCount = items.filter((item) => statuses.includes(item.status)).length;
  const toggle = (status: WorkStatus, on: boolean) =>
    setStatuses((current) => WORK_STATUS_ORDER.filter((s) => s === status ? on : current.includes(s)));
  const run = async (task: () => Promise<void>) => {
    setBusy(true); setNote(null); setPending(null);
    try { await task(); } finally { setBusy(false); }
  };
  const onExport = () => run(async () => {
    const { json, count } = serializeWorkItems(items, statuses);
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
    if (parsed.ok) setPending(parsed);
    else setNote({ text: t(`transfer.errors.${parsed.error}`), error: true });
  });
  const onConfirm = () => {
    if (!pending) return;
    const added = useWorkItemStore.getState().importFile(pending.items);
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
      <p className="work-muted">{t("transfer.exportHint")}</p>
      <div className="work-dialog-actions">
        <span className="work-dialog-spacer" />
        <button className="pixel-btn primary" disabled={busy || !exportCount} onClick={() => void onExport()}>{t("transfer.export", { count: exportCount })}</button>
      </div>
    </section>
    <section className="work-transfer-section work-dialog-run">
      <h3>{t("transfer.importHeading")}</h3>
      <p className="work-muted">{t("transfer.importHint")}</p>
      {storeError && <p className="work-send-hint">{t("transfer.importBlocked")}</p>}
      {pending ? <>
        <p>{t("transfer.importPreview", { count: pending.items.length })}{pending.skipped > 0 && ` ${t("transfer.importSkipped", { count: pending.skipped })}`}</p>
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
