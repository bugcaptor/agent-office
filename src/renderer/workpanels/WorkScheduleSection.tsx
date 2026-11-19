import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { WorkItem, WorkScheduleRule } from "@shared/types";
import { formatLocalMinute } from "../shared/localDateTime";
import { useWorkItemStore } from "./workItemStore";
import { planItems } from "./autoRun";
import { skipText } from "./scheduleRunner";
import { disabledSchedule, enabledSchedule, localDateTime, SCHEDULE_MAX_MINUTES, SCHEDULE_MIN_MINUTES, validScheduleRule } from "./workSchedule";

type Kind = WorkScheduleRule["kind"];
type Unit = "m" | "h" | "d";
const UNIT_MINUTES: Record<Unit, number> = { m: 1, h: 60, d: 1440 };
/** 월요일부터 보여 준다. 값은 0=일요일. */
const DAY_ORDER = [1, 2, 3, 4, 5, 6, 0];

type TFn = (key: string, options?: Record<string, unknown>) => string;

function weekdayName(day: number): string {
  // 2023-01-01은 일요일이다.
  return new Date(2023, 0, 1 + day).toLocaleDateString([], { weekday: "short" });
}

/** 규칙 한 줄 요약. 카드 칩과 CLI 표시와 같은 내용이다. */
export function ruleText(t: TFn, rule: WorkScheduleRule): string {
  switch (rule.kind) {
    case "once": return t("schedule.rule.once", { when: formatLocalMinute(rule.at) });
    case "interval": {
      const unit: Unit = rule.minutes % 1440 === 0 ? "d" : rule.minutes % 60 === 0 ? "h" : "m";
      return t(`schedule.rule.${unit === "d" ? "everyDays" : unit === "h" ? "everyHours" : "everyMinutes"}`, { count: rule.minutes / UNIT_MINUTES[unit] });
    }
    case "daily": return t("schedule.rule.daily", { time: rule.time });
    case "weekly": return t("schedule.rule.weekly", { days: DAY_ORDER.filter((d) => rule.days.includes(d)).map(weekdayName).join(", "), time: rule.time });
  }
}

interface Draft { kind: Kind; date: string; time: string; amount: number; unit: Unit; days: number[] }

function draftOf(rule: WorkScheduleRule | undefined, now: number): Draft {
  const soon = formatLocalMinute(now + 3600_000).split(" ");
  const base: Draft = { kind: "daily", date: soon[0], time: "09:00", amount: 1, unit: "h", days: [1, 2, 3, 4, 5] };
  if (!rule) return base;
  switch (rule.kind) {
    case "once": { const [date, time] = formatLocalMinute(rule.at).split(" "); return { ...base, kind: "once", date, time }; }
    case "interval": {
      const unit: Unit = rule.minutes % 1440 === 0 ? "d" : rule.minutes % 60 === 0 ? "h" : "m";
      return { ...base, kind: "interval", amount: rule.minutes / UNIT_MINUTES[unit], unit };
    }
    case "daily": return { ...base, kind: "daily", time: rule.time };
    case "weekly": return { ...base, kind: "weekly", days: rule.days, time: rule.time };
  }
}

function ruleOf(draft: Draft): WorkScheduleRule | undefined {
  switch (draft.kind) {
    case "once": { const at = localDateTime(draft.date, draft.time); return at === undefined ? undefined : { kind: "once", at }; }
    case "interval": return Number.isFinite(draft.amount) ? { kind: "interval", minutes: Math.round(draft.amount * UNIT_MINUTES[draft.unit]) } : undefined;
    case "daily": return { kind: "daily", time: draft.time };
    case "weekly": return { kind: "weekly", days: [...draft.days].sort((a, b) => a - b), time: draft.time };
  }
}

/** 업무 상세 팝업의 예약 영역. 켜고 끄기·규칙 바꾸기는 바로 저장한다(편집기 초안과 따로). */
export function WorkScheduleSection({ item }: { item: WorkItem }) {
  const { t } = useTranslation("workpanels");
  const items = useWorkItemStore((s) => s.items);
  const setSchedule = useWorkItemStore((s) => s.setSchedule);
  const schedule = item.schedule;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft>(() => draftOf(schedule?.rule, Date.now()));
  // 예약 회차(복제본)에는 새 예약을 걸지 않는다. 회차가 또 템플릿이 되면 헷갈린다.
  if (!schedule && (item.status === "done" || item.scheduledFrom)) return null;
  const plan = planItems(item, items).length > 0;
  const rule = ruleOf(draft);
  const now = Date.now();
  const valid = !!rule && validScheduleRule(rule, now);
  const invalidHint = !rule || valid ? undefined
    : rule.kind === "once" ? t("schedule.invalid.past")
      : rule.kind === "interval" ? t("schedule.invalid.interval", { min: SCHEDULE_MIN_MINUTES, maxDays: SCHEDULE_MAX_MINUTES / 1440 })
        : rule.kind === "weekly" && !rule.days.length ? t("schedule.invalid.days") : t("schedule.invalid.time");
  const save = () => {
    const next = rule && enabledSchedule(rule, Date.now(), schedule);
    if (!next) return;
    setSchedule(item.id, next);
    setEditing(false);
    void useWorkItemStore.getState().flush();
  };
  const update = (patch: Partial<Draft>) => setDraft((current) => ({ ...current, ...patch }));
  const persist = (next: typeof schedule) => { setSchedule(item.id, next); void useWorkItemStore.getState().flush(); };
  const canEnable = !!schedule && !schedule.enabled && validScheduleRule(schedule.rule, now) && item.status !== "done";
  const recurring = (editing ? draft.kind : schedule?.rule.kind) !== "once" && (editing || !!schedule);
  return <section className="work-schedule" aria-labelledby={`schedule-${item.id}`}>
    <h3 id={`schedule-${item.id}`}>{t("schedule.title")}</h3>
    {!schedule && !editing && <p className="work-muted">{t("schedule.none")}</p>}
    {schedule && <>
      <p><strong>{ruleText(t, schedule.rule)}</strong>{" · "}
        {schedule.enabled
          ? <span>{schedule.nextAt !== undefined ? t("schedule.next", { when: formatLocalMinute(schedule.nextAt) }) : t("schedule.on")}</span>
          : <span className="work-blocked">{t("schedule.off")}</span>}</p>
      {!schedule.enabled && <p className="work-muted">{t(item.status === "done" ? "schedule.offDone" : canEnable ? "schedule.offHint" : "schedule.offInvalid")}</p>}
      {schedule.lastRunAt !== undefined && <p className="work-muted">{t("schedule.lastRun", { when: formatLocalMinute(schedule.lastRunAt) })}</p>}
      {schedule.lastSkip && <p className="work-muted">{t("schedule.lastSkip", { when: formatLocalMinute(schedule.lastSkip.at), reason: skipText(schedule.lastSkip) })}</p>}
    </>}
    {(editing || schedule) && <p className="work-muted">{t(recurring ? (plan ? "schedule.recurringPlanHint" : "schedule.recurringHint") : (plan ? "schedule.oncePlanHint" : "schedule.onceHint"))}</p>}
    {editing && <div className="work-schedule-editor">
      <label>{t("schedule.kind")}<select value={draft.kind} onChange={(e) => update({ kind: e.target.value as Kind })}>
        {(["once", "interval", "daily", "weekly"] as const).map((kind) => <option key={kind} value={kind}>{t(`schedule.kinds.${kind}`)}</option>)}
      </select></label>
      {draft.kind === "once" && <label>{t("schedule.when")}<input type="datetime-local" value={`${draft.date}T${draft.time}`}
        onChange={(e) => { const [date, time = ""] = e.target.value.split("T"); update({ date, time: time.slice(0, 5) }); }} /></label>}
      {draft.kind === "interval" && <div className="work-schedule-row">
        <label>{t("schedule.every")}<input type="number" min={1} step={1} value={Number.isFinite(draft.amount) ? draft.amount : ""} onChange={(e) => update({ amount: e.target.valueAsNumber })} /></label>
        <select aria-label={t("schedule.unit")} value={draft.unit} onChange={(e) => update({ unit: e.target.value as Unit })}>
          {(["m", "h", "d"] as const).map((unit) => <option key={unit} value={unit}>{t(`schedule.units.${unit}`)}</option>)}
        </select>
      </div>}
      {draft.kind === "weekly" && <div className="work-schedule-days" role="group" aria-label={t("schedule.days")}>
        {DAY_ORDER.map((day) => <label key={day}><input type="checkbox" checked={draft.days.includes(day)}
          onChange={(e) => update({ days: e.target.checked ? [...draft.days, day] : draft.days.filter((d) => d !== day) })} />{weekdayName(day)}</label>)}
      </div>}
      {(draft.kind === "daily" || draft.kind === "weekly") && <label>{t("schedule.time")}<input type="time" value={draft.time} onChange={(e) => update({ time: e.target.value })} /></label>}
      {invalidHint && <p className="work-send-hint" role="alert">{invalidHint}</p>}
    </div>}
    <div className="work-dialog-actions">
      {schedule && !editing && <button type="button" className="pixel-btn" onClick={() => persist(undefined)}>{t("schedule.remove")}</button>}
      {editing && <button type="button" className="pixel-btn" onClick={() => setEditing(false)}>{t("cancel")}</button>}
      <span className="work-dialog-spacer" />
      {!editing && item.status !== "done" && <button type="button" className="pixel-btn" onClick={() => { setDraft(draftOf(schedule?.rule, Date.now())); setEditing(true); }}>{t(schedule ? "schedule.change" : "schedule.add")}</button>}
      {schedule?.enabled && !editing && <button type="button" className="pixel-btn" onClick={() => persist(disabledSchedule(schedule))}>{t("schedule.turnOff")}</button>}
      {canEnable && !editing && <button type="button" className="pixel-btn primary" onClick={() => persist(enabledSchedule(schedule.rule, Date.now(), schedule))}>{t("schedule.turnOn")}</button>}
      {editing && <button type="button" className="pixel-btn primary" disabled={!valid} onClick={save}>{t("schedule.saveOn")}</button>}
    </div>
  </section>;
}
