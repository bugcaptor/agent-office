import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

/** 상세·편집 팝업. 헤더 ×, Esc, 배경 클릭은 초안을 유지한 채 닫는다. */
export function WorkDialog({ title, onClose, children }: { title: string; onClose(): void; children: ReactNode }) {
  const { t } = useTranslation("workpanels");
  const ref = useRef<HTMLElement>(null);
  // 헤더의 ×가 아니라 첫 입력란에 포커스를 둔다.
  useEffect(() => { ref.current?.querySelector<HTMLElement>("input, textarea, select")?.focus(); }, []);
  return createPortal(<div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}
    onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } }}>
    <section ref={ref} className="pixel-panel work-detail-dialog" role="dialog" aria-modal="true" aria-labelledby="work-detail-title">
      <header className="work-dialog-header">
        <h2 id="work-detail-title">{title}</h2>
        <button type="button" className="pixel-btn work-dialog-close" aria-label={t("collapseWork")} title={t("collapseWork")} onClick={onClose}>×</button>
      </header>
      {children}
    </section>
  </div>, document.body);
}
