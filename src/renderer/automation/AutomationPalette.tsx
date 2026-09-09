import {
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import { useTranslation } from "react-i18next";
import type {
  AutomationDefinition,
  AutomationStep,
  AutomationRunRecord,
} from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { backendErrorText } from "../shared/backendError";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { useAppStore } from "../store/appStore";
import "./automation.css";

const id = () => crypto.randomUUID();
const launch = (): AutomationStep => ({
  id: id(),
  kind: "launchCli",
  cliProfileId: "claude",
  startupWaitMs: 10_000,
});
const blank = (): AutomationDefinition => ({
  schemaVersion: 1,
  id: id(),
  revision: 0,
  name: "",
  inputs: [],
  steps: [],
  inputValues: {},
});
const replace = (text: string, values: Record<string, string>) =>
  text.replace(/{{([^{}]+)}}/g, (_, key) => values[key] ?? `{{${key}}}`);
const shellQuote = (value: string) => `'${value.replace(/'/g, "'\\''")}'`;
function launchCommand(step: Extract<AutomationStep, { kind: "launchCli" }>) {
  const model = step.model ? ` --model ${shellQuote(step.model)}` : "";
  if (step.cliProfileId === "codex")
    return `codex${model}${step.effort ? ` -c ${shellQuote(`model_reasoning_effort="${step.effort}"`)}` : ""}`;
  if (step.cliProfileId === "agy")
    return `agy --dangerously-skip-permissions${model}${step.effort ? ` --effort ${shellQuote(step.effort)}` : ""}`;
  if (step.cliProfileId === "kilo") return `kilo --auto${model}`;
  if (step.cliProfileId === "pi")
    return `pi --approve${model}${step.effort ? ` --thinking ${shellQuote(step.effort)}` : ""}`;
  return `claude${model}${step.effort ? ` --effort ${shellQuote(step.effort)}` : ""}`;
}
type TransitionPreview = {
  shellPath?: string;
  autoReturnSupported: boolean;
  unavailableReason?: string;
  launchCommand?: string;
};

type ModelCatalogs = Record<string, { models: string[]; failed: boolean }>;

function automationStartError(
  error: unknown,
  t: (key: string, options?: Record<string, string>) => string,
) {
  const reason = backendErrorText(error);
  const unavailable = "automation-agy-model-unavailable:";
  if (reason.startsWith(unavailable))
    return t("automation.agyModelUnavailable", {
      model: reason.slice(unavailable.length),
    });
  if (reason === "automation-agy-model-catalog-unavailable")
    return t("automation.agyModelCatalogUnavailable");
  if (reason === "automation-agy-effort-invalid")
    return t("automation.agyEffortInvalid");
  if (reason === "automation-session-changed")
    return t("automation.errSessionChanged");
  return t("automation.editorStartFailedReason", { reason });
}

function automationSaveError(
  error: unknown,
  t: (key: string, options?: Record<string, string>) => string,
) {
  const reason = backendErrorText(error);
  if (reason === "automation-agy-effort-invalid")
    return t("automation.editorSaveFailed", {
      reason: t("automation.agyEffortInvalid"),
    });
  return t("automation.editorSaveFailed", { reason });
}

function examples(t: (key: string) => string): AutomationDefinition[] {
  const input = {
    key: "task",
    label: t("automation.exampleTask"),
    default: t("automation.exampleTaskDefault"),
  };
  const task = (label: string, prompt: string): AutomationStep => ({
    id: id(),
    kind: "llmTask",
    label,
    promptTemplate: prompt,
    waitTimeoutMs: 1_800_000,
    completionGraceMs: 10_000,
  });
  return [
    {
      ...blank(),
      schemaVersion: 2,
      name: t("automation.sampleClaudeAgy"),
      inputs: [input],
      steps: [
        launch(),
        task(t("automation.sampleDesign"), t("automation.exampleClaudeDesign")),
        {
          id: id(),
          kind: "exitCli",
          command: "/exit",
          returnMode: "auto",
          exitWaitMs: 30_000,
        },
        {
          ...(launch() as Extract<AutomationStep, { kind: "launchCli" }>),
          cliProfileId: "agy",
        },
        task(
          t("automation.sampleImplement"),
          t("automation.exampleAgyImplement"),
        ),
        {
          id: id(),
          kind: "exitCli",
          command: "/exit",
          returnMode: "auto",
          exitWaitMs: 30_000,
        },
      ],
    },
    {
      ...blank(),
      name: t("automation.sampleSingle"),
      inputs: [input],
      steps: [
        launch(),
        task(t("automation.sampleTask"), t("automation.examplePrompt")),
      ],
    },
    {
      ...blank(),
      name: t("automation.sampleWriteReview"),
      inputs: [input],
      steps: [
        launch(),
        task(t("automation.sampleWrite"), t("automation.examplePrompt")),
        task(t("automation.sampleReview"), t("automation.exampleReview")),
      ],
    },
    {
      ...blank(),
      name: t("automation.sampleImplementReview"),
      inputs: [input],
      repeat: { maxCycles: 2 },
      steps: [
        launch(),
        task(t("automation.sampleImplement"), t("automation.examplePrompt")),
        task(t("automation.sampleReview"), t("automation.exampleImprove")),
        { id: id(), kind: "exitCli", command: "/exit" },
      ],
    },
  ];
}
const exitCommand = (cliProfileId?: string) =>
  cliProfileId === "pi" ? "/quit" : "/exit";

function step(
  kind: AutomationStep["kind"],
  t: (key: string) => string,
  precedingCliProfileId?: string,
): AutomationStep {
  if (kind === "launchCli") return launch();
  if (kind === "llmTask")
    return {
      id: id(),
      kind,
      label: t("automation.defaultTask"),
      promptTemplate: "",
      waitTimeoutMs: 1_800_000,
      completionGraceMs: 10_000,
    };
  if (kind === "wait") return { id: id(), kind, durationMs: 1_000 };
  if (kind === "confirm")
    return { id: id(), kind, message: t("automation.defaultConfirm") };
  return { id: id(), kind, command: exitCommand(precedingCliProfileId) };
}

export function AutomationPalette() {
  const { t } = useTranslation("terminal");
  const editor = useAppStore((s) => s.automationEditor);
  const close = useAppStore((s) => s.closeAutomationEditor);
  const seed = useAppStore((s) => s.seedAutomationStatus);
  const sessions = useAppStore((s) => s.sessions);
  const automation = useAppStore((s) => s.automation);
  const bot = useAppStore((s) => s.botMode);
  const file = useRef<HTMLInputElement>(null);
  const inFlight = useRef(false);
  const [defs, setDefs] = useState<AutomationDefinition[]>([]);
  const [runs, setRuns] = useState<AutomationRunRecord[]>([]);
  const [current, setCurrent] = useState<AutomationDefinition | null>(null);
  const [saved, setSaved] = useState<AutomationDefinition | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [transitionPreviews, setTransitionPreviews] = useState<
    Record<string, TransitionPreview>
  >({});
  const [modelCatalogs, setModelCatalogs] = useState<ModelCatalogs>({});
  const [modelRefresh, setModelRefresh] = useState(0);
  const modelProfilesKey = [
    ...new Set(
      (current?.steps ?? []).flatMap((step) =>
        step.kind === "launchCli" ? [step.cliProfileId] : [],
      ),
    ),
  ]
    .sort()
    .join(",");
  const loadGeneration = useRef(0);
  useEscapeToClose(!!editor, close);
  const refresh = async (generation = loadGeneration.current) => {
    const [definitions, history] = await Promise.all([
      tauriApi.automationDefinitionsList(),
      tauriApi.automationRunsList(),
    ]);
    if (generation !== loadGeneration.current) return;
    setDefs(definitions);
    setRuns(history);
  };
  useEffect(() => {
    const generation = ++loadGeneration.current;
    setCurrent(null);
    setSaved(null);
    setValues({});
    setNotice(null);
    setBusy(false);
    if (editor) {
      void refresh(generation).catch(
        () =>
          generation === loadGeneration.current &&
          setNotice(t("automation.editorLoadFailed")),
      );
    }
  }, [editor]);
  useEffect(() => {
    const launchStep =
      current?.schemaVersion === 2
        ? current.steps.find(
            (s): s is Extract<AutomationStep, { kind: "launchCli" }> =>
              s.kind === "launchCli",
          )
        : undefined;
    if (!editor || !launchStep) {
      setTransitionPreviews({});
      return;
    }
    let alive = true;
    const launches = (current?.steps ?? []).filter(
      (s): s is Extract<AutomationStep, { kind: "launchCli" }> =>
        s.kind === "launchCli",
    );
    void Promise.all(
      launches.map(
        async (step) =>
          [
            step.id,
            await tauriApi.automationCliTransitionPreview(
              editor.agentId,
              step.cliProfileId,
              step.model,
              step.effort,
            ),
          ] as const,
      ),
    )
      .then(
        (entries) =>
          alive && setTransitionPreviews(Object.fromEntries(entries)),
      )
      .catch(
        () =>
          alive &&
          setTransitionPreviews(
            Object.fromEntries(
              launches.map((step) => [
                step.id,
                {
                  autoReturnSupported: false,
                  unavailableReason: "automation-cli-return-shell-unverified",
                },
              ]),
            ),
          ),
      );
    return () => {
      alive = false;
    };
  }, [editor, current?.schemaVersion, current?.steps]);
  useEffect(() => {
    setModelCatalogs({});
    if (!editor || !modelProfilesKey) return;
    let alive = true;
    for (const cli of modelProfilesKey.split(",")) {
      void tauriApi.automationCliModels(cli).then(
        (models) => {
          if (alive)
            setModelCatalogs((catalogs) => ({
              ...catalogs,
              [cli]: { models, failed: models.length === 0 },
            }));
        },
        () => {
          if (alive)
            setModelCatalogs((catalogs) => ({
              ...catalogs,
              [cli]: { models: [], failed: true },
            }));
        },
      );
    }
    return () => {
      alive = false;
    };
  }, [editor, modelProfilesKey, modelRefresh]);
  const resolvedInputs = (definition: AutomationDefinition) =>
    Object.fromEntries(
      definition.inputs.map((input) => [
        input.key,
        values[input.key] ?? input.default ?? "",
      ]),
    );
  const select = (d: AutomationDefinition) => {
    setCurrent(d);
    setSaved(d);
    setValues(
      Object.fromEntries(
        d.inputs.map((x) => [x.key, d.inputValues?.[x.key] ?? x.default ?? ""]),
      ),
    );
  };
  const update = (f: (d: AutomationDefinition) => AutomationDefinition) =>
    setCurrent((d) => d && f(d));
  const persistedInputs = (definition: AutomationDefinition) =>
    Object.fromEntries(
      definition.inputs.map((input) => [
        input.key,
        definition.inputValues?.[input.key] ?? input.default ?? "",
      ]),
    );
  const dirty =
    !!current &&
    (!saved ||
      JSON.stringify(current) !== JSON.stringify(saved) ||
      JSON.stringify(resolvedInputs(current)) !==
        JSON.stringify(persistedInputs(saved)));
  const save = async () => {
    if (!current || inFlight.current) return;
    const generation = loadGeneration.current;
    inFlight.current = true;
    setBusy(true);
    try {
      const result = await tauriApi.automationDefinitionsSave({
        ...current,
        inputValues: resolvedInputs(current),
      });
      if (generation !== loadGeneration.current) return;
      setCurrent(result);
      setSaved(result);
      setDefs((old) => [...old.filter((x) => x.id !== result.id), result]);
      setNotice(t("automation.editorSaved"));
    } catch (error) {
      if (generation !== loadGeneration.current) return;
      setNotice(automationSaveError(error, t));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };
  const run = async () => {
    if (!editor || !saved || dirty || inFlight.current) {
      if (dirty) setNotice(t("automation.editorSaveBeforeRun"));
      return;
    }
    if (
      automation[editor.agentId]?.running ||
      bot[editor.agentId] ||
      !["starting", "running"].includes(
        sessions[editor.agentId]?.status ?? "idle",
      )
    ) {
      setNotice(t("automation.editorRunUnavailable"));
      return;
    }
    inFlight.current = true;
    const generation = loadGeneration.current;
    const automationEpoch =
      useAppStore.getState().automationEpochs[editor.agentId] ?? 0;
    setBusy(true);
    try {
      const status = await tauriApi.automationRunStart(
        editor.agentId,
        saved.id,
        resolvedInputs(saved),
      );
      if (generation !== loadGeneration.current) return;
      seed(
        { agents: { [editor.agentId]: status } },
        { [editor.agentId]: automationEpoch },
      );
      close();
    } catch (error) {
      setNotice(automationStartError(error, t));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };
  const importJson = async (json: string) => {
    if (inFlight.current) return;
    inFlight.current = true;
    const generation = loadGeneration.current;
    setBusy(true);
    try {
      const result = await tauriApi.automationDefinitionsImport(json);
      if (generation !== loadGeneration.current) return;
      select(result);
      await refresh(generation);
    } catch {
      setNotice(t("automation.editorImportFailed"));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };
  const deleteDefinition = async (definition: AutomationDefinition) => {
    if (inFlight.current) return;
    inFlight.current = true;
    const generation = loadGeneration.current;
    setBusy(true);
    try {
      await tauriApi.automationDefinitionsDelete(definition.id);
      if (generation !== loadGeneration.current) return;
      setDefs((old) => old.filter((item) => item.id !== definition.id));
      if (current?.id === definition.id) {
        setCurrent(null);
        setSaved(null);
        setValues({});
      }
    } catch {
      setNotice(t("automation.editorDeleteFailed"));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };
  if (!editor) return null;
  const previewValues = current
    ? {
        ...resolvedInputs(current),
        workspace: editor.workspace,
        run: "<run>",
        cycle: "1",
        previousResult: t("automation.previewPreviousResult"),
      }
    : {};
  const previewDuration = (ms: number) =>
    ms >= 1_000
      ? t("automation.previewSeconds", { value: (ms / 1_000).toFixed(1) })
      : t("automation.previewMilliseconds", { value: ms });
  const preview =
    current?.steps
      .map((s, index) => {
        if (s.kind === "llmTask")
          return `${index + 1}. ${s.label}: ${replace(s.promptTemplate, previewValues)} (${previewDuration(s.waitTimeoutMs ?? 1_800_000)}; +${previewDuration(s.completionGraceMs ?? 10_000)}${s.allowEarlyComplete ? `; ${t("automation.fieldEarlyComplete")}` : ""})`;
        if (s.kind === "launchCli")
          return `${index + 1}. ${t("automation.step.launchCli")}: ${current?.schemaVersion === 2 ? (transitionPreviews[s.id]?.launchCommand ?? t("automation.previewCommandUnavailable")) : launchCommand(s)} (${previewDuration(s.startupWaitMs ?? 10_000)})`;
        if (s.kind === "exitCli")
          return `${index + 1}. ${t("automation.step.exitCli")}: ${s.command}${current?.schemaVersion === 2 ? ` — ${t(s.returnMode === "manual" ? "automation.previewReturnManual" : "automation.previewReturnAuto", { time: previewDuration(s.exitWaitMs ?? 30_000) })}` : ""}`;
        if (s.kind === "wait")
          return `${index + 1}. ${t("automation.step.wait")}: ${previewDuration(s.durationMs)}`;
        return `${index + 1}. ${t("automation.step.confirm")}: ${replace(s.message, previewValues)}`;
      })
      .join("\n") ?? "";
  const repeatPreview = current?.repeat
    ? `\n${t(current.schemaVersion === 2 ? "automation.previewTransitionRepeat" : "automation.previewRepeat", { count: current.repeat.maxCycles })}`
    : "";
  const transitionPreview = Object.values(transitionPreviews).find(
    (preview) => !preview.autoReturnSupported,
  );
  const transitionUnavailable =
    transitionPreview?.unavailableReason ===
    "automation-cli-return-shell-unsupported"
      ? t("automation.transitionShellUnsupported")
      : transitionPreview?.unavailableReason ===
          "automation-cli-return-shell-unverified"
        ? t("automation.transitionShellUnverified")
        : undefined;
  return (
    <div
      className="automation-overlay"
      role="presentation"
      onMouseDown={(e) => e.target === e.currentTarget && close()}
    >
      <section
        className="automation-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t("automation.editorTitle")}
      >
        <header>
          <div>
            <h2>{t("automation.editorTitle")}</h2>
            <code>{editor.workspace}</code>
          </div>
          <button
            type="button"
            onClick={close}
            aria-label={t("automation.editorClose")}
          >
            ×
          </button>
        </header>
        <div className="automation-columns">
          <aside>
            <div className="automation-actions">
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  if (inFlight.current) return;
                  setCurrent(blank());
                  setSaved(null);
                  setValues({});
                }}
              >
                {t("automation.editorNew")}
              </button>
              {examples(t).map((x) => (
                <button
                  type="button"
                  key={x.id}
                  disabled={busy}
                  onClick={() => {
                    if (inFlight.current) return;
                    setCurrent(x);
                    setSaved(null);
                    setValues(
                      Object.fromEntries(
                        x.inputs.map((input) => [
                          input.key,
                          x.inputValues?.[input.key] ?? input.default ?? "",
                        ]),
                      ),
                    );
                  }}
                >
                  {x.name}
                </button>
              ))}
              <button
                type="button"
                disabled={busy}
                onClick={() => file.current?.click()}
              >
                {t("automation.editorImport")}
              </button>
              <input
                ref={file}
                type="file"
                accept="application/json"
                hidden
                onChange={(e) => {
                  const selected = e.target.files?.[0];
                  if (selected)
                    void selected
                      .text()
                      .then(importJson)
                      .catch(() =>
                        setNotice(t("automation.editorImportFailed")),
                      );
                  e.target.value = "";
                }}
              />
            </div>
            <ul className="automation-definition-list">
              {defs.map((d) => (
                <li key={d.id}>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => select(d)}
                    className={current?.id === d.id ? "selected" : ""}
                  >
                    {d.name}
                  </button>
                  <button
                    type="button"
                    aria-label={t("automation.editorCopy")}
                    disabled={busy}
                    onClick={() => {
                      if (inFlight.current) return;
                      setCurrent({
                        ...d,
                        id: id(),
                        revision: 0,
                        name: `${d.name} ${t("automation.editorCopySuffix")}`,
                        steps: d.steps.map((x) => ({ ...x, id: id() })),
                      });
                      setSaved(null);
                      setValues(
                        Object.fromEntries(
                          d.inputs.map((input) => [
                            input.key,
                            d.inputValues?.[input.key] ?? input.default ?? "",
                          ]),
                        ),
                      );
                    }}
                  >
                    ⧉
                  </button>
                  <button
                    type="button"
                    aria-label={t("automation.editorDelete")}
                    disabled={busy}
                    onClick={() => void deleteDefinition(d)}
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
            <h3>{t("automation.editorHistory")}</h3>
            <ul className="automation-history">
              {runs.slice(0, 8).map((r) => (
                <li key={r.runId}>
                  {r.definitionSnapshot.name} ·{" "}
                  {t(`automation.history.${r.status}`, {
                    defaultValue: r.status,
                  })}
                </li>
              ))}
            </ul>
          </aside>
          <main>
            {current ? (
              <Editor
                d={current}
                t={t}
                update={update}
                values={values}
                setValues={setValues}
                workspace={editor.workspace}
                preview={`${preview}${repeatPreview}`}
                busy={busy}
                transitionUnavailable={
                  current.schemaVersion === 2 &&
                  transitionPreview?.autoReturnSupported === false
                    ? transitionUnavailable
                    : undefined
                }
                modelCatalogs={modelCatalogs}
                refreshModels={() => setModelRefresh((n) => n + 1)}
              />
            ) : (
              <p>{t("automation.editorEmpty")}</p>
            )}
          </main>
        </div>
        {notice && (
          <p className="automation-notice" role="status">
            {notice}
          </p>
        )}
        {current && (
          <footer>
            <button type="button" disabled={busy} onClick={() => void save()}>
              {t("automation.editorSave")}
            </button>
            <button
              type="button"
              disabled={
                busy ||
                dirty ||
                (current.schemaVersion === 2 &&
                  current.steps.some(
                    (step) =>
                      step.kind === "launchCli" &&
                      !transitionPreviews[step.id]?.autoReturnSupported,
                  ))
              }
              onClick={() => void run()}
            >
              {t("automation.editorRun")}
            </button>
            <button
              type="button"
              disabled={busy || dirty}
              onClick={() =>
                void tauriApi
                  .automationDefinitionsExport(saved!.id)
                  .then((json) => {
                    const a = document.createElement("a");
                    a.href = URL.createObjectURL(new Blob([json]));
                    a.download = `${saved!.name}.json`;
                    a.click();
                    URL.revokeObjectURL(a.href);
                  })
                  .catch(() => setNotice(t("automation.editorExportFailed")))
              }
            >
              {t("automation.editorExport")}
            </button>
          </footer>
        )}
      </section>
    </div>
  );
}

function Editor({
  d,
  t,
  update,
  values,
  setValues,
  workspace,
  preview,
  busy,
  transitionUnavailable,
  modelCatalogs,
  refreshModels,
}: {
  d: AutomationDefinition;
  t: (key: string, options?: any) => string;
  update: (f: (d: AutomationDefinition) => AutomationDefinition) => void;
  values: Record<string, string>;
  setValues: Dispatch<SetStateAction<Record<string, string>>>;
  workspace: string;
  preview: string;
  busy: boolean;
  transitionUnavailable?: string;
  modelCatalogs: ModelCatalogs;
  refreshModels: () => void;
}) {
  const move = (i: number, by: number) =>
    update((x) => {
      const steps = [...x.steps],
        at = i + by;
      if (at < 0 || at >= steps.length) return x;
      [steps[i], steps[at]] = [steps[at], steps[i]];
      return { ...x, steps };
    });
  return (
    <fieldset disabled={busy}>
      <label>
        {t("automation.editorFlowType")}
        <select
          value={d.schemaVersion}
          onChange={(e) =>
            update((x) => ({
              ...x,
              schemaVersion: Number(e.target.value) as 1 | 2,
              steps:
                Number(e.target.value) === 1
                  ? x.steps.map((s) =>
                      s.kind === "exitCli"
                        ? { id: s.id, kind: s.kind, command: s.command }
                        : s,
                    )
                  : x.steps,
            }))
          }
        >
          <option value={1}>{t("automation.flowSingle")}</option>
          <option value={2}>{t("automation.flowTransition")}</option>
        </select>
        <small>
          {t(
            d.schemaVersion === 2
              ? "automation.flowTransitionDesc"
              : "automation.flowSingleDesc",
          )}
        </small>
      </label>
      <label>
        {t("automation.editorName")}
        <input
          value={d.name}
          onChange={(e) => update((x) => ({ ...x, name: e.target.value }))}
        />
      </label>
      <label>
        {t("menu.workdir")}
        <code>{workspace}</code>
        <small>{t("automation.editorWorkspaceNote")}</small>
      </label>
      <label>
        {t("automation.editorRepeat")}
        <input
          type="number"
          min="1"
          value={d.repeat?.maxCycles ?? ""}
          onChange={(e) =>
            update((x) => ({
              ...x,
              repeat: e.target.value
                ? { maxCycles: Number(e.target.value) }
                : undefined,
            }))
          }
        />
      </label>
      <h3>{t("automation.editorInputs")}</h3>
      {d.inputs.map((input, i) => (
        <div className="automation-row" key={i}>
          <input
            aria-label={t("automation.editorInputKey")}
            value={input.key}
            onChange={(e) => {
              const key = e.target.value;
              setValues((current) => {
                const next = { ...current };
                if (key !== input.key) {
                  delete next[input.key];
                  next[key] = input.default ?? "";
                }
                return next;
              });
              update((x) => ({
                ...x,
                inputs: x.inputs.map((v, n) => (n === i ? { ...v, key } : v)),
              }));
            }}
          />
          <input
            aria-label={t("automation.editorInputLabel")}
            value={input.label}
            onChange={(e) =>
              update((x) => ({
                ...x,
                inputs: x.inputs.map((v, n) =>
                  n === i ? { ...v, label: e.target.value } : v,
                ),
              }))
            }
          />
          <input
            aria-label={t("automation.editorInputDefault")}
            value={input.default ?? ""}
            onChange={(e) =>
              update((x) => ({
                ...x,
                inputs: x.inputs.map((v, n) =>
                  n === i ? { ...v, default: e.target.value || undefined } : v,
                ),
              }))
            }
          />
          <button
            type="button"
            aria-label={t("automation.editorDelete")}
            onClick={() =>
              update((x) => ({
                ...x,
                inputs: x.inputs.filter((_, n) => n !== i),
              }))
            }
          >
            ×
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={() =>
          update((x) => ({
            ...x,
            inputs: [
              ...x.inputs,
              {
                key: `input${x.inputs.length + 1}`,
                label: t("automation.defaultInput"),
              },
            ],
          }))
        }
      >
        {t("automation.editorAddInput")}
      </button>
      <h3>{t("automation.editorSteps")}</h3>
      <div className="automation-step-add">
        {(["launchCli", "llmTask", "wait", "confirm", "exitCli"] as const).map(
          (kind) => (
            <button
              type="button"
              key={kind}
              onClick={() =>
                update((x) => ({
                  ...x,
                  steps: [
                    ...x.steps,
                    step(
                      kind,
                      t,
                      [...x.steps]
                        .reverse()
                        .find(
                          (candidate): candidate is Extract<AutomationStep, { kind: "launchCli" }> =>
                            candidate.kind === "launchCli",
                        )?.cliProfileId,
                    ),
                  ],
                }))
              }
            >
              {t(`automation.step.${kind}`)}
            </button>
          ),
        )}
      </div>
      {d.steps.map((s, i) => (
        <Step
          key={s.id}
          step={s}
          t={t}
          change={(next) =>
            update((x) => {
              const cliTransition =
                s.kind === "launchCli" &&
                next.kind === "launchCli" &&
                s.cliProfileId !== next.cliProfileId
                  ? {
                      from: exitCommand(s.cliProfileId),
                      to: exitCommand(next.cliProfileId),
                    }
                  : undefined;
              let updateDefaultExit = !!cliTransition;
              return {
                ...x,
                steps: x.steps.map((v, n) => {
                  if (n === i) return next;
                  if (n <= i) return v;
                  if (v.kind === "launchCli") updateDefaultExit = false;
                  if (updateDefaultExit && v.kind === "exitCli") {
                    updateDefaultExit = false;
                    return v.command === cliTransition!.from
                      ? { ...v, command: cliTransition!.to }
                      : v;
                  }
                  return v;
                }),
              };
            })
          }
          move={(by) => move(i, by)}
          copy={() =>
            update((x) => ({
              ...x,
              steps: [
                ...x.steps.slice(0, i + 1),
                { ...s, id: id() },
                ...x.steps.slice(i + 1),
              ],
            }))
          }
          remove={() =>
            update((x) => ({ ...x, steps: x.steps.filter((_, n) => n !== i) }))
          }
          isV2={d.schemaVersion === 2}
          modelCatalogs={modelCatalogs}
          refreshModels={refreshModels}
        />
      ))}
      <h3>{t("automation.editorPreview")}</h3>
      <pre>{preview}</pre>
      {transitionUnavailable && (
        <p className="automation-notice">
          {t("automation.transitionUnavailable", {
            reason: transitionUnavailable,
          })}
        </p>
      )}
      {d.inputs.map((x) => (
        <label key={x.key}>
          {x.label}
          <input
            value={values[x.key] ?? x.default ?? ""}
            onChange={(e) => setValues({ ...values, [x.key]: e.target.value })}
          />
        </label>
      ))}
    </fieldset>
  );
}

function Step({
  step: s,
  t,
  change,
  move,
  copy,
  remove,
  isV2,
  modelCatalogs,
  refreshModels,
}: {
  step: AutomationStep;
  t: (key: string, options?: Record<string, string>) => string;
  change: (step: AutomationStep) => void;
  move: (n: number) => void;
  copy: () => void;
  remove: () => void;
  isV2: boolean;
  modelCatalogs: ModelCatalogs;
  refreshModels: () => void;
}) {
  const input = (
    label: string,
    value: string | number,
    onChange: (value: string) => void,
    number = false,
  ) => (
    <label>
      {label}
      <input
        type={number ? "number" : "text"}
        min={number ? 1 : undefined}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );
  return (
    <section className="automation-step">
      <div>
        <strong>{t(`automation.step.${s.kind}`)}</strong>
        <button
          type="button"
          aria-label={t("automation.editorMoveUp")}
          onClick={() => move(-1)}
        >
          ↑
        </button>
        <button
          type="button"
          aria-label={t("automation.editorMoveDown")}
          onClick={() => move(1)}
        >
          ↓
        </button>
        <button
          type="button"
          aria-label={t("automation.editorCopy")}
          onClick={copy}
        >
          ⧉
        </button>
        <button
          type="button"
          aria-label={t("automation.editorDelete")}
          onClick={remove}
        >
          ×
        </button>
      </div>
      {s.kind === "launchCli" && (
        <>
          <label>
            {t("automation.fieldCli")}
            <select
              value={s.cliProfileId}
              onChange={(e) =>
                change({
                  ...s,
                  cliProfileId: e.target.value,
                  model: undefined,
                  effort: undefined,
                })
              }
            >
              <option value="claude">Claude</option>
              <option value="codex">Codex</option>
              <option value="agy">{t("automation.cliAgy")}</option>
              <option value="kilo">{t("automation.cliKilo")}</option>
              <option value="pi">{t("automation.cliPi")}</option>
            </select>
          </label>
          <label>
            {t("automation.fieldModel")}
            <input
              aria-label={t("automation.fieldModel")}
              value={s.model ?? ""}
              onChange={(e) =>
                change({ ...s, model: e.target.value || undefined })
              }
            />
            <select
              aria-label={t("automation.modelCandidates", {
                cli: s.cliProfileId,
              })}
              value=""
              disabled={!modelCatalogs[s.cliProfileId]?.models.length}
              onChange={(e) =>
                e.target.value && change({ ...s, model: e.target.value })
              }
            >
              <option value="">
                {t("automation.modelCandidates", { cli: s.cliProfileId })}
              </option>
              {(modelCatalogs[s.cliProfileId]?.models ?? []).map((model) => (
                <option key={model} value={model}>
                  {model}
                </option>
              ))}
            </select>
            <small>
              {modelCatalogs[s.cliProfileId]?.failed
                ? t("automation.modelCatalogLoadFailed", {
                    cli: s.cliProfileId,
                  })
                : !modelCatalogs[s.cliProfileId]
                  ? t("automation.modelCatalogLoading", { cli: s.cliProfileId })
                  : t("automation.modelCatalogHint", { cli: s.cliProfileId })}
            </small>
            {modelCatalogs[s.cliProfileId]?.failed && (
              <button type="button" onClick={refreshModels}>
                {t("automation.modelCatalogRetry")}
              </button>
            )}
          </label>
          {s.cliProfileId !== "kilo" &&
            input(t("automation.fieldEffort"), s.effort ?? "", (effort) =>
              change({ ...s, effort: effort || undefined }),
            )}
          {input(
            t("automation.fieldStartupMs"),
            s.startupWaitMs ?? 10_000,
            (v) => change({ ...s, startupWaitMs: Number(v) }),
            true,
          )}
        </>
      )}
      {s.kind === "llmTask" && (
        <>
          {input(t("automation.fieldLabel"), s.label, (label) =>
            change({ ...s, label }),
          )}
          <label>
            {t("automation.fieldPrompt")}
            <textarea
              value={s.promptTemplate}
              onChange={(e) => change({ ...s, promptTemplate: e.target.value })}
            />
          </label>
          {input(
            t("automation.fieldWaitMs"),
            s.waitTimeoutMs ?? 1_800_000,
            (v) => change({ ...s, waitTimeoutMs: Number(v) }),
            true,
          )}
          {input(
            t("automation.fieldGraceMs"),
            s.completionGraceMs ?? 10_000,
            (v) => change({ ...s, completionGraceMs: Number(v) }),
            true,
          )}
          <label>
            <input
              type="checkbox"
              checked={s.allowEarlyComplete ?? false}
              onChange={(e) =>
                change({ ...s, allowEarlyComplete: e.target.checked })
              }
            />
            {t("automation.fieldEarlyComplete")}
          </label>
        </>
      )}
      {s.kind === "wait" &&
        input(
          t("automation.fieldDurationMs"),
          s.durationMs,
          (v) => change({ ...s, durationMs: Number(v) }),
          true,
        )}
      {s.kind === "confirm" &&
        input(t("automation.fieldMessage"), s.message, (message) =>
          change({ ...s, message }),
        )}
      {s.kind === "exitCli" && (
        <>
          {input(t("automation.fieldCommand"), s.command, (command) =>
            change({ ...s, command }),
          )}
          {isV2 && (
            <>
              <label>
                {t("automation.fieldReturnMode")}
                <select
                  value={s.returnMode ?? "auto"}
                  onChange={(e) =>
                    change({
                      ...s,
                      returnMode: e.target.value as "auto" | "manual",
                    })
                  }
                >
                  <option value="auto">{t("automation.returnAuto")}</option>
                  <option value="manual">{t("automation.returnManual")}</option>
                </select>
              </label>
              {(s.returnMode ?? "auto") === "auto" &&
                input(
                  t("automation.fieldExitWaitMs"),
                  s.exitWaitMs ?? 30_000,
                  (v) => change({ ...s, exitWaitMs: Number(v) }),
                  true,
                )}
            </>
          )}
        </>
      )}
    </section>
  );
}
