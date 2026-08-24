// src/renderer/workdir/WorkdirRepoLogPane.tsx
//
// 저장소 전체 커밋 로그 브라우저(이슈 #54, 2단계). 파일을 먼저 지목하지 않고
// 로그 → 커밋 → 변경파일 → diff 순으로 훑는다. 좌측 커밋 목록(검색·전체브랜치·
// 더 보기), 우측은 선택 커밋의 변경파일 목록과 고른 파일의 그 커밋 diff.
//
// 검색 입력은 매 타건마다 git을 때리지 않도록 300ms 디바운스로 스토어 쿼리에
// 반영한다(스토어가 쿼리 변경 시 첫 페이지부터 재조회).
//
// 조회 중단 UI(타임아웃 개편): 거대 저장소의 로그/변경파일/ diff 조회는 분 단위가
// 될 수 있어, 모든 "불러오는 중…" 자리에 취소 버튼을 두고 취소된 결과에는 "다시
// 시도"를 붙인다(상세 페인과 같은 관례).
//
// i18n: 커밋·변경파일 어휘("이 커밋이 바꾼 파일" 등)와 diff 본문 문구는 상세
// 페인과 글자 그대로 같으므로 `detail.*`/`diff.*` 키를 그대로 재사용한다 —
// 같은 문구를 두 벌 두면 번역이 갈라진다.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useWorkdirStore } from "./workdirStore";
import { DiffView } from "./DiffView";
import { statusLabel } from "./status";
import type { GitDiffResult } from "@shared/types";

/** 진행/중단 안내 한 줄(취소·다시 시도 버튼 포함) — 상세 페인과 같은 모양. */
function BusyNote({
  text,
  actionLabel,
  onAction,
}: {
  text: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  return (
    <div className="wd-detail-empty">
      <span>{text}</span>
      {actionLabel && onAction && (
        <button type="button" className="wd-btn wd-btn-mini" onClick={onAction}>
          {actionLabel}
        </button>
      )}
    </div>
  );
}

/** 경로의 마지막 세그먼트(파일명). */
function basename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

/** diff 본문(상세 페인과 동일한 상태 처리 + 취소/재시도). */
function DiffBody({
  diff,
  loading,
  onCancel,
  onRetry,
}: {
  diff?: GitDiffResult;
  loading: boolean;
  onCancel?: () => void;
  onRetry?: () => void;
}) {
  const { t } = useTranslation("workdir");
  const aborted = !!diff && (diff.canceled || diff.timedOut);
  if (loading && (!diff || aborted))
    return (
      <BusyNote text={t("diff.loading")} actionLabel={t("palette.cancel")} onAction={onCancel} />
    );
  if (!diff) return <div className="wd-detail-empty">{t("diff.selectFile")}</div>;
  if (diff.canceled)
    return (
      <BusyNote text={t("diff.canceled")} actionLabel={t("palette.retry")} onAction={onRetry} />
    );
  if (diff.timedOut)
    return (
      <BusyNote text={t("diff.timedOut")} actionLabel={t("palette.retry")} onAction={onRetry} />
    );
  if (diff.binary) return <div className="wd-detail-empty">{t("diff.binary")}</div>;
  if (diff.diff.trim() === "") return <div className="wd-detail-empty">{t("diff.empty")}</div>;
  return (
    <>
      {diff.truncated && <div className="wd-note">{t("diff.truncated")}</div>}
      <DiffView diff={diff.diff} />
    </>
  );
}

export function WorkdirRepoLogPane() {
  const { t } = useTranslation("workdir");
  const root = useWorkdirStore((s) => s.palette?.root ?? "");
  const rl = useWorkdirStore((s) => (s.palette ? s.repoLog[s.palette.root] : undefined));
  const loadRepoLog = useWorkdirStore((s) => s.loadRepoLog);
  const setRepoLogQuery = useWorkdirStore((s) => s.setRepoLogQuery);
  const setRepoLogAllBranches = useWorkdirStore((s) => s.setRepoLogAllBranches);
  const selectRepoCommit = useWorkdirStore((s) => s.selectRepoCommit);
  const loadMoreRepoFiles = useWorkdirStore((s) => s.loadMoreRepoFiles);
  const selectRepoFile = useWorkdirStore((s) => s.selectRepoFile);
  const openRepoDifftool = useWorkdirStore((s) => s.openRepoDifftool);
  const closePalette = useWorkdirStore((s) => s.closePalette);
  const cancelOp = useWorkdirStore((s) => s.cancelOp);

  // 검색 입력: 로컬 상태 + 디바운스로 스토어 쿼리에 반영.
  const appliedQuery = rl?.query ?? "";
  const [text, setText] = useState(appliedQuery);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // 외부(다른 root 재오픈 등)에서 쿼리가 바뀌면 입력도 동기화.
  useEffect(() => {
    setText(appliedQuery);
  }, [appliedQuery, root]);
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);

  const onSearchChange = (v: string) => {
    setText(v);
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      if (v !== (useWorkdirStore.getState().repoLog[root]?.query ?? "")) setRepoLogQuery(v);
    }, 300);
  };

  const commits = rl?.commits;
  const selectedCommit = rl?.selectedCommit;

  return (
    <div className="wd-log">
      <div className="wd-log-toolbar">
        <input
          className="wd-input wd-log-search"
          type="text"
          placeholder={t("repoLog.searchPlaceholder")}
          value={text}
          spellCheck={false}
          onChange={(e) => onSearchChange(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Escape") {
              e.preventDefault();
              closePalette();
            }
          }}
        />
        <label className="wd-git-toggle" title={t("repoLog.allBranchesTitle")}>
          <input
            type="checkbox"
            checked={rl?.allBranches ?? false}
            onChange={(e) => setRepoLogAllBranches(e.target.checked)}
          />
          <span>{t("repoLog.allBranches")}</span>
        </label>
      </div>

      <div className="wd-log-body">
        {/* 좌: 커밋 목록 */}
        <div className="wd-log-commits">
          {rl?.timedOut && (
            <div className="wd-note">{t("repoLog.timedOut")}</div>
          )}
          {commits === undefined && rl?.loading ? (
            <BusyNote
              text={t("repoLog.loading")}
              actionLabel={t("palette.cancel")}
              onAction={() => cancelOp(rl?.opId)}
            />
          ) : commits === undefined && rl?.canceled ? (
            <BusyNote
              text={t("diff.canceled")}
              actionLabel={t("palette.retry")}
              onAction={() => void loadRepoLog(true)}
            />
          ) : commits === undefined ? (
            <div className="wd-empty">{t("repoLog.loading")}</div>
          ) : commits.length === 0 ? (
            <div className="wd-empty">
              {appliedQuery ? t("repoLog.emptyMatch") : t("repoLog.empty")}
            </div>
          ) : (
            <ul className="wd-history" role="listbox" aria-label={t("detail.commitListAria")}>
              {commits.map((c) => (
                <li
                  key={c.hash}
                  role="option"
                  aria-selected={selectedCommit === c.hash}
                  className={
                    selectedCommit === c.hash ? "wd-commit wd-commit-active" : "wd-commit"
                  }
                >
                  <div className="wd-commit-main" onClick={() => selectRepoCommit(c.hash)}>
                    <span className="wd-commit-hash">{c.shortHash}</span>
                    <span className="wd-commit-subject">{c.subject}</span>
                    <span className="wd-commit-meta">
                      {c.author} · {c.date}
                    </span>
                  </div>
                </li>
              ))}
              {rl?.hasMore && (
                <li className="wd-cf-more" onClick={() => loadRepoLog(false)}>
                  {rl?.loading ? t("palette.loadingMore") : t("palette.loadMore")}
                </li>
              )}
            </ul>
          )}
        </div>

        {/* 우: 선택 커밋의 변경파일 + 고른 파일 diff */}
        <div className="wd-log-detail">
          {!selectedCommit ? (
            <div className="wd-detail-empty">{t("repoLog.selectCommit")}</div>
          ) : (
            <>
              <ul className="wd-commit-files wd-log-files" aria-label={t("detail.commitFilesAria")}>
                {rl?.filesLoading && !rl?.files ? (
                  <li className="wd-cf-note">
                    {t("detail.commitFilesLoading")}{" "}
                    <button
                      type="button"
                      className="wd-btn wd-btn-mini"
                      onClick={() => cancelOp(rl?.filesOpId)}
                    >
                      {t("palette.cancel")}
                    </button>
                  </li>
                ) : rl?.filesCanceled && !rl?.files ? (
                  <li className="wd-cf-note">
                    {t("diff.canceled")}{" "}
                    <button
                      type="button"
                      className="wd-btn wd-btn-mini"
                      onClick={() => void selectRepoCommit(selectedCommit)}
                    >
                      {t("palette.retry")}
                    </button>
                  </li>
                ) : (rl?.files ?? []).length === 0 ? (
                  <li className="wd-cf-note">{t("detail.commitFilesEmpty")}</li>
                ) : (
                  <>
                    {(rl?.files ?? []).map((f) => (
                      <li
                        key={f.path}
                        className={rl?.selectedFile === f.path ? "wd-cf wd-cf-active" : "wd-cf"}
                        title={f.path}
                        onClick={() => selectRepoFile(selectedCommit, f.path)}
                      >
                        <span
                          className={`wd-badge wd-badge-${f.status}`}
                          title={statusLabel(f.status, t)}
                          aria-label={statusLabel(f.status, t)}
                        >
                          {f.status}
                        </span>
                        <span className="wd-cf-name">{basename(f.path)}</span>
                        <span className="wd-cf-path">{f.path}</span>
                      </li>
                    ))}
                    {rl?.filesHasMore && (
                      <li className="wd-cf-more" onClick={() => loadMoreRepoFiles()}>
                        {rl?.filesLoading ? t("palette.loadingMore") : t("palette.loadMore")}
                      </li>
                    )}
                  </>
                )}
              </ul>
              {rl?.selectedFile && (
                <div className="wd-commit-diff">
                  <div className="wd-detail-actions">
                    <span className="wd-cf-difflabel" title={rl.selectedFile}>
                      {basename(rl.selectedFile)}
                    </span>
                    <button
                      type="button"
                      className="wd-btn"
                      title={t("detail.difftoolTitle")}
                      onClick={() => openRepoDifftool()}
                    >
                      {t("detail.difftool")}
                    </button>
                  </div>
                  <DiffBody
                    diff={rl.fileDiff}
                    loading={rl.fileDiffLoading}
                    onCancel={() => cancelOp(rl.fileDiffOpId)}
                    onRetry={() => void selectRepoFile(selectedCommit, rl.selectedFile!)}
                  />
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
