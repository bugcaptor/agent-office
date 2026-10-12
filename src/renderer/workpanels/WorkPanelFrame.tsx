import { useTranslation } from "react-i18next";
import type { ReactNode } from "react";
import { usePanelLayoutStore, type PanelSide } from "./panelLayoutStore";

export function WorkPanelFrame({ side, title, children }: { side: PanelSide; title: string; children: ReactNode }) {
  const { t } = useTranslation("workpanels");
  const panel = usePanelLayoutStore((s) => s.panels[side]);
  const update = usePanelLayoutStore((s) => s.update);
  return <aside className={`work-panel work-panel-${side}${panel.collapsed ? " collapsed" : ""}`} aria-label={title}>
    <header>
      {!panel.collapsed && <strong>{title}</strong>}
      <button className="pixel-btn" title={t(panel.collapsed ? "expand" : "collapse")} aria-label={t(panel.collapsed ? "expand" : "collapse")} onClick={() => update(side, { collapsed: !panel.collapsed })}>
        {side === "left" ? (panel.collapsed ? "›" : "‹") : (panel.collapsed ? "‹" : "›")}
      </button>
    </header>
    {!panel.collapsed && <>
      <div className="work-panel-content">{children}</div>
      <div className="work-panel-resize" role="separator" tabIndex={0} aria-label={t("resize")} aria-orientation="vertical" aria-valuemin={220} aria-valuemax={480} aria-valuenow={panel.width}
        onKeyDown={(e) => {
          if (e.key === "ArrowLeft" || e.key === "ArrowRight") { e.preventDefault(); update(side, { width: panel.width + (e.key === "ArrowRight" ? 10 : -10) * (side === "left" ? 1 : -1) }); }
        }}
        onPointerDown={(e) => { if (e.button === 0) { e.currentTarget.setPointerCapture(e.pointerId); e.currentTarget.dataset.startX = String(e.clientX); e.currentTarget.dataset.startWidth = String(panel.width); } }}
        onPointerMove={(e) => { if (e.currentTarget.hasPointerCapture(e.pointerId)) update(side, { width: Number(e.currentTarget.dataset.startWidth) + (e.clientX - Number(e.currentTarget.dataset.startX)) * (side === "left" ? 1 : -1) }); }}
        onPointerUp={(e) => { if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId); }} />
    </>}
  </aside>;
}
