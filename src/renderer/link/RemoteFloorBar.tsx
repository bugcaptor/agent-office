// src/renderer/link/RemoteFloorBar.tsx
//
// 머신 연동 호스트: 오피스 위의 층 단추(이 머신 · 붙은 머신들)와 다른 층 캐릭터 hover 카드. 붙은 머신이
// 없으면(호스트가 아니거나 페어링한 기기가 없으면) 아무것도 그리지 않는다.
import { useTranslation } from "react-i18next";
import { findRemote, machineSummary, selectFloor, useRemoteFloors } from "./remoteFloors";
import { folderName } from "../workpanels/sessionRows";
import "./remoteFloors.css";

export function RemoteFloorBar() {
  const { t } = useTranslation("office");
  const office = useRemoteFloors((s) => s.office);
  const floor = useRemoteFloors((s) => s.floor);
  if (!office.machines.length) return null;
  const current = office.machines.find((m) => m.deviceId === floor);
  return (
    <div className="remote-floors">
      <div className="remote-floor-tabs pixel-panel" role="tablist" aria-label={t("floors.aria")}>
        <button type="button" role="tab" aria-selected={!floor} className={`remote-floor-tab${!floor ? " active" : ""}`} onClick={() => selectFloor(null)}>
          {t("floors.local")}
        </button>
        {office.machines.map((m) => {
          const { working, waiting } = machineSummary(m);
          const selected = m.deviceId === floor;
          return (
            <button
              key={m.deviceId}
              type="button"
              role="tab"
              aria-selected={selected}
              className={`remote-floor-tab${selected ? " active" : ""}${m.online ? "" : " offline"}`}
              title={m.online ? t("floors.summary", { working, waiting }) : t("floors.offline")}
              onClick={() => selectFloor(m.deviceId)}
            >
              <span className={`remote-floor-dot${m.online ? " on" : ""}`} aria-hidden="true" />
              {m.name}
              {m.online && waiting > 0 && <span className="remote-floor-wait">{t("floors.waiting", { count: waiting })}</span>}
            </button>
          );
        })}
      </div>
      {current && !current.online && <div className="remote-floor-note">{t("floors.offlineNote")}</div>}
      {current && current.online && !current.characters.length && <div className="remote-floor-note">{t("floors.empty")}</div>}
    </div>
  );
}

/** 다른 층 캐릭터에 올린 포인터 옆의 이름·시작 폴더·상태. */
export function RemoteHoverCard() {
  const { t } = useTranslation("office");
  const hover = useRemoteFloors((s) => s.hover);
  const office = useRemoteFloors((s) => s.office);
  const found = hover && findRemote(office, hover.id);
  if (!hover || !found) return null;
  const { machine, character: c } = found;
  const state = !machine.online
    ? "offline"
    : c.needsHuman
      ? "waiting"
      : c.sessionState === "running" || c.sessionState === "starting"
        ? "working"
        : c.sessionState
          ? "idle"
          : "off";
  return (
    <div className="remote-hover-card pixel-panel" style={{ left: hover.x + 12, top: hover.y + 12 }}>
      <div className="remote-hover-name">{c.name}</div>
      {/* 층이 곧 머신이라 머신 이름 대신 그 캐릭터의 시작 폴더 말단 이름을 보인다. */}
      <div className="remote-hover-sub" title={c.cwd}>{[folderName(c.cwd), c.role].filter(Boolean).join(" · ")}</div>
      <div className={`remote-hover-state ${state}`}>{t(`floors.state.${state}`)}</div>
      <div className="remote-hover-hint">{t("floors.openChat")}</div>
    </div>
  );
}
