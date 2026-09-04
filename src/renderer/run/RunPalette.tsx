import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { RunRecipe, RunRecipeProcess } from "@shared/types";

import { tauriApi } from "../ipc/tauriApi";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { useAppStore } from "../store/appStore";
import { executeRunRecipe, probeRunRecipes } from "./execute";
import { useRunStore } from "./runStore";
import "./run.css";

function RecipeRow({
  recipe,
  disabled,
  onRun,
  onDelete,
}: {
  recipe: RunRecipe;
  disabled: boolean;
  onRun: () => void;
  onDelete?: () => void;
}) {
  const { t } = useTranslation("run");
  return (
    <li>
      <button
        type="button"
        className="run-recipe-row"
        disabled={disabled}
        title={recipe.note || recipe.command}
        onClick={onRun}
      >
        <span className={`run-source run-source-${recipe.source}`}>
          {t(`source.${recipe.source}`)}
        </span>
        {recipe.longRunning && (
          <span className="run-long">{t("recipe.longRunning")}</span>
        )}
        <span className="run-recipe-main">
          <strong>{recipe.label}</strong>
          <code>{recipe.command}</code>
        </span>
        <span className="run-action">{t("recipe.run")}</span>
      </button>
      {onDelete && (
        <button
          type="button"
          className="run-delete"
          disabled={disabled}
          aria-label={t("recipe.deleteAria", { label: recipe.label })}
          onClick={onDelete}
        >
          ×
        </button>
      )}
    </li>
  );
}

export function RunPalette() {
  const { t } = useTranslation("run");
  const palette = useRunStore((s) => s.palette);
  const result = useRunStore((s) => s.result);
  const loading = useRunStore((s) => s.loading);
  const saving = useRunStore((s) => s.saving);
  const error = useRunStore((s) => s.error);
  const closePalette = useRunStore((s) => s.closePalette);
  const refresh = useRunStore((s) => s.refresh);
  const addUserRecipe = useRunStore((s) => s.addUserRecipe);
  const deleteUserRecipe = useRunStore((s) => s.deleteUserRecipe);
  const clearAgentRecipes = useRunStore((s) => s.clearAgentRecipes);
  const enabled = useAppStore((s) => s.appSettings.runRecipesEnabled);
  const turn = useAppStore((s) =>
    palette ? s.timeTracking[palette.agentId] : undefined,
  );
  const [label, setLabel] = useState("");
  const [command, setCommand] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [acting, setActing] = useState(false);
  const [runningProcess, setRunningProcess] =
    useState<RunRecipeProcess | null>(null);
  const actionInFlight = useRef(false);
  const processStatusGeneration = useRef(0);
  const lastSettledTurns = useRef<number | undefined>(undefined);
  const refreshAfterLoad = useRef(false);

  useEffect(() => {
    if (palette && !enabled) closePalette();
  }, [palette, enabled, closePalette]);
  useEscapeToClose(palette !== null && enabled, closePalette);

  // 전용 실행 프로세스는 팔레트를 닫아도 계속 돈다. 다시 열었을 때와 자연
  // 종료 뒤 상태를 맞추려고, 열린 동안만 가볍게 조회한다.
  useEffect(() => {
    if (!palette) {
      processStatusGeneration.current += 1;
      setRunningProcess(null);
      return;
    }
    let disposed = false;
    const poll = async () => {
      const generation = ++processStatusGeneration.current;
      try {
        const process = await tauriApi.runRecipeStatus(palette.agentId);
        if (!disposed && generation === processStatusGeneration.current) {
          setRunningProcess(process);
        }
      } catch (statusError) {
        console.warn("RunPalette: process status failed", statusError);
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [palette]);

  // notification Stop이나 shell idle이 turns를 올리면 캐릭터가 방금 쓴 파일을 다시 읽는다.
  useEffect(() => {
    if (!palette) {
      lastSettledTurns.current = undefined;
      refreshAfterLoad.current = false;
      return;
    }
    const turns = turn?.turns ?? 0;
    if (lastSettledTurns.current === undefined) {
      lastSettledTurns.current = turns;
      return;
    }
    if (turns !== lastSettledTurns.current) {
      lastSettledTurns.current = turns;
      if (result) void refresh();
      else refreshAfterLoad.current = true;
      return;
    }
    if (result && refreshAfterLoad.current) {
      refreshAfterLoad.current = false;
      void refresh();
    }
  }, [palette, refresh, result, turn?.turns]);

  if (!palette || !enabled) return null;

  const run = async (recipe: RunRecipe) => {
    if (actionInFlight.current) return;
    actionInFlight.current = true;
    setActing(true);
    setNotice(null);
    try {
      const outcome = await executeRunRecipe(
        palette.agentId,
        result?.root ?? palette.root,
        recipe,
      );
      if (outcome === "started") {
        const generation = ++processStatusGeneration.current;
        try {
          const process = await tauriApi.runRecipeStatus(palette.agentId);
          if (generation === processStatusGeneration.current) {
            setRunningProcess(process);
          }
        } catch (statusError) {
          console.warn("RunPalette: process status failed", statusError);
        }
      }
      setNotice(t(`notice.${outcome}`));
    } finally {
      actionInFlight.current = false;
      setActing(false);
    }
  };

  const stop = async () => {
    if (actionInFlight.current) return;
    actionInFlight.current = true;
    setActing(true);
    setNotice(null);
    try {
      await tauriApi.runRecipeStop(palette.agentId);
      processStatusGeneration.current += 1;
      setRunningProcess(null);
      setNotice(t("notice.stopped"));
    } catch (stopError) {
      console.warn("RunPalette: process stop failed", stopError);
      setNotice(t("notice.stopFailed"));
    } finally {
      actionInFlight.current = false;
      setActing(false);
    }
  };

  const probe = async () => {
    if (actionInFlight.current) return;
    actionInFlight.current = true;
    setActing(true);
    setNotice(null);
    try {
      const outcome = await probeRunRecipes(palette.agentId, palette.root);
      setNotice(t(`notice.probe.${outcome}`));
    } catch (probeError) {
      console.warn("RunPalette: probe failed", probeError);
      setNotice(t("notice.probe.failed"));
    } finally {
      actionInFlight.current = false;
      setActing(false);
    }
  };

  const submitManual = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!label.trim() || !command.trim()) return;
    if (await addUserRecipe(label, command)) {
      setLabel("");
      setCommand("");
    }
  };

  const busy = loading || saving || acting;
  return (
    <div
      className="run-overlay"
      role="presentation"
      onMouseDown={(e) => e.target === e.currentTarget && closePalette()}
    >
      <section
        className="run-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t("title")}
        onKeyDown={(event) => event.stopPropagation()}
      >
        <header className="run-header">
          <div>
            <h2>{t("title")}</h2>
            <code>{palette.root}</code>
          </div>
          <button
            type="button"
            className="run-close"
            aria-label={t("close")}
            onClick={closePalette}
          >
            ×
          </button>
        </header>

        {runningProcess && (
          <div className="run-current">
            <span>{t("current.title")}</span>
            <code title={runningProcess.command}>
              {runningProcess.label} · {runningProcess.command}
            </code>
            <button
              type="button"
              disabled={busy}
              onClick={() => void stop()}
            >
              {t("current.stop")}
            </button>
          </div>
        )}

        <div className="run-scroll">
          <section className="run-section">
            <h3>{t("section.agent")}</h3>
            {loading && !result ? (
              <p className="run-empty">{t("state.loading")}</p>
            ) : result?.agentState === "missing" ? (
              <p className="run-empty">{t("state.missing")}</p>
            ) : result?.agentState === "invalid" ? (
              <p className="run-empty run-error">{t("state.invalid")}</p>
            ) : result?.agentRecipes.length ? (
              <ul className="run-list">
                {result.agentRecipes.map((recipe) => (
                  <RecipeRow
                    key={recipe.id}
                    recipe={recipe}
                    disabled={busy || runningProcess !== null}
                    onRun={() => void run(recipe)}
                  />
                ))}
              </ul>
            ) : (
              <p className="run-empty">{t("state.emptyAgent")}</p>
            )}
          </section>

          <section className="run-section">
            <h3>{t("section.user")}</h3>
            {result?.userRecipes.length ? (
              <ul className="run-list">
                {result.userRecipes.map((recipe) => (
                  <RecipeRow
                    key={recipe.id}
                    recipe={recipe}
                    disabled={busy || runningProcess !== null}
                    onRun={() => void run(recipe)}
                    onDelete={() => void deleteUserRecipe(recipe.id)}
                  />
                ))}
              </ul>
            ) : (
              <p className="run-empty">{t("state.emptyUser")}</p>
            )}
            <form
              className="run-add"
              onSubmit={(event) => void submitManual(event)}
            >
              <input
                value={label}
                disabled={busy || !result}
                onChange={(e) => setLabel(e.target.value)}
                placeholder={t("add.label")}
              />
              <input
                value={command}
                disabled={busy || !result}
                onChange={(e) => setCommand(e.target.value)}
                placeholder={t("add.command")}
                spellCheck={false}
              />
              <button
                type="submit"
                disabled={busy || !result || !label.trim() || !command.trim()}
              >
                {t("add.submit")}
              </button>
            </form>
          </section>
        </div>

        {(error || notice) && (
          <p className={error ? "run-notice run-error" : "run-notice"}>
            {error ? t("state.error") : notice}
          </p>
        )}
        <footer className="run-footer">
          <button type="button" disabled={busy} onClick={() => void probe()}>
            {t("footer.probe")}
          </button>
          <button type="button" disabled={busy} onClick={() => void refresh()}>
            {t("footer.refresh")}
          </button>
          <button
            type="button"
            disabled={busy || result?.agentState === "missing"}
            onClick={() => void clearAgentRecipes()}
          >
            {t("footer.clear")}
          </button>
        </footer>
      </section>
    </div>
  );
}
