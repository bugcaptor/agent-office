import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

/** 복사 결과를 버튼에 보이는 시간. */
const COPY_FEEDBACK_MS = 1500;

/** 클립보드 복사. 지원하지 않는 환경(jsdom/구형)에서는 false를 돌려준다. */
async function writeClipboard(text: string): Promise<boolean> {
  try {
    if (typeof navigator === "undefined" || !navigator.clipboard) return false;
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** 태스크 ID와 복사 버튼. ID만 복사하고 결과를 1.5초 동안 버튼에 보인다. */
function WorkDialogId({ id }: { id: string }) {
  const { t } = useTranslation("workpanels");
  const [copy, setCopy] = useState<"idle" | "copied" | "failed">("idle");
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const onCopy = async () => {
    const ok = await writeClipboard(id);
    clearTimeout(timer.current);
    setCopy(ok ? "copied" : "failed");
    timer.current = setTimeout(() => setCopy("idle"), COPY_FEEDBACK_MS);
  };
  const label = copy === "copied" ? t("idCopied") : copy === "failed" ? t("idCopyFailed") : t("copyId");
  return <span className="work-dialog-id">
    <code>{id}</code>
    <button type="button" className={`pixel-btn work-dialog-copy ${copy}`} aria-label={label} title={label} onClick={() => void onCopy()}>
      {copy === "copied" ? "✓" : copy === "failed" ? "!" : "⧉"}
    </button>
  </span>;
}

/** 상세·편집 팝업. 헤더 ×, Esc, 배경 클릭은 초안을 유지한 채 닫는다. `id`가 있으면(기존 업무) 제목 옆에 ID와 복사 버튼을 둔다. */
export function WorkDialog({ title, id, onClose, children }: { title: string; id?: string; onClose(): void; children: ReactNode }) {
  const { t } = useTranslation("workpanels");
  const ref = useRef<HTMLElement>(null);
  // 헤더의 ×가 아니라 첫 입력란에 포커스를 둔다.
  useEffect(() => { ref.current?.querySelector<HTMLElement>("input, textarea, select")?.focus(); }, []);
  return createPortal(<div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}
    onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } }}>
    <section ref={ref} className="pixel-panel work-detail-dialog" role="dialog" aria-modal="true" aria-labelledby="work-detail-title">
      <header className="work-dialog-header">
        <h2 id="work-detail-title">{title}</h2>
        {id && <WorkDialogId id={id} />}
        <button type="button" className="pixel-btn work-dialog-close" aria-label={t("collapseWork")} title={t("collapseWork")} onClick={onClose}>×</button>
      </header>
      {children}
    </section>
  </div>, document.body);
}
