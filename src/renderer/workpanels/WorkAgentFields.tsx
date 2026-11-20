import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { WorkAgent } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { workAgentLabel } from "../profile/generate";

const CLIS: WorkAgent["cli"][] = ["claude", "codex", "agy", "kilo", "pi"];
const EFFORTS: Record<WorkAgent["cli"], string[]> = {
  claude: ["low", "medium", "high", "xhigh", "max"], codex: ["low", "medium", "high", "xhigh", "max"],
  agy: ["low", "medium", "high"], kilo: [], pi: ["off", "minimal", "low", "medium", "high", "xhigh", "max"],
};
/** 저장만 하는 편집기. 모델 후보 조회는 실행하지 않으며 수동 입력을 보존한다. */
export function WorkAgentFields({ value, onChange, profile = false, inherited, labels }: {
  value?: WorkAgent; onChange(value: WorkAgent | undefined): void; profile?: boolean; inherited?: WorkAgent;
  /** 칸 이름과 "미지정" 항목 문구를 바꾼다(번역된 문자열). 주면 실효값 안내를 숨긴다. */
  labels?: { field: string; empty: string };
}) {
  const { t } = useTranslation("workpanels");
  const id = useId();
  const [models, setModels] = useState<string[]>([]);
  const [catalog, setCatalog] = useState<"loading" | "failed" | "ready">("ready");
  const [retry, setRetry] = useState(0);
  const cli = value?.cli;
  useEffect(() => {
    let alive = true;
    setModels([]);
    if (!cli || cli === "kilo") { setCatalog("ready"); return; }
    setCatalog("loading");
    void tauriApi.workAgentModels(cli).then((list) => {
      if (alive) { setModels(list); setCatalog(list.length ? "ready" : "failed"); }
    }, () => { if (alive) setCatalog("failed"); });
    return () => { alive = false; };
  }, [cli, retry]);
  const effective = inherited ?? { cli: "claude" };
  return <div className="work-agent-fields">
    <label><span className="form-label-text">{labels?.field ?? t(profile ? "defaultAgent" : "agentOverride")}</span><select value={cli ?? ""} onChange={(e) => onChange(e.target.value ? { cli: e.target.value as WorkAgent["cli"] } : undefined)}>
      <option value="">{labels?.empty ?? t(profile ? "claudeDefault" : "useProfileDefault")}</option>
      {CLIS.map((cli) => <option value={cli} key={cli}>{cli}</option>)}
    </select></label>
    {!value && !profile && !labels && <small>{t("effectiveAgent", { agent: workAgentLabel(effective) })}</small>}
    {value && <>
      <label><span className="form-label-text">{t("model")}</span><input list={id} value={value.model ?? ""} placeholder={t("cliDefault")} onChange={(e) => onChange({ ...value, model: e.target.value || undefined })} /></label>
      <datalist id={id}>{models.map((model) => <option value={model} key={model} />)}</datalist>
      {catalog === "loading" && <small>{t("modelsLoading")}</small>}
      {catalog === "failed" && <small>{t("modelsFailed")} <button type="button" className="pixel-btn" onClick={() => setRetry((v) => v + 1)}>{t("retry")}</button></small>}
      {value.cli !== "kilo" && <label><span className="form-label-text">{t("effort")}</span><select value={value.effort ?? ""} onChange={(e) => onChange({ ...value, effort: e.target.value || undefined })}>
        <option value="">{t("cliDefault")}</option>
        {value.effort && !EFFORTS[value.cli].includes(value.effort) && <option value={value.effort}>{value.effort}</option>}
        {EFFORTS[value.cli].map((effort) => <option key={effort} value={effort}>{effort}</option>)}
      </select></label>}
    </>}
  </div>;
}
