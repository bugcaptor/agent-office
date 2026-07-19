// src/renderer/workdir/WorkdirDetailPane.tsx
//
// 작업 폴더 보기의 우측 상세(메뉴) 페인. 이슈 #54로 **모든** 파일이 여기로 들어와
// 열기 전에 깃 로그·변경점을 먼저 볼 수 있다. 상단에 "외부 프로그램으로 열기"·(마크
// 다운 등) "인앱 뷰어로 열기" 버튼. 두 탭:
//   - 변경점: 워킹트리↔HEAD(전체)·인덱스↔HEAD(스테이지됨)·워킹트리↔인덱스
//     (미스테이지) 세 관점 전환(미추적 파일은 단일 뷰). "외부 도구로 비교" 버튼.
//   - 히스토리: `git log --follow` 커밋 목록. 커밋을 펼치면(▸) 그 커밋이 바꾼 파일
//     목록을 인라인으로 보여주고(더 보기…로 페이징), 파일을 고르면 그 커밋의 해당
//     파일 diff를 하단에 띄운다. 펼치지 않고 커밋만 고르면 지금 파일의 그 커밋 diff.
import { useWorkdirStore, isMarkdownPath } from "./workdirStore";
import { DiffView } from "./DiffView";
import { statusLabel } from "./status";
import type { GitDiffMode, GitDiffResult } from "@shared/types";

/** diff 모드 → 탭 라벨(추적 파일용 3탭). */
const MODE_TABS: { mode: GitDiffMode; label: string; title: string }[] = [
  { mode: "worktreeVsHead", label: "전체", title: "워킹트리 ↔ HEAD(스테이지+미스테이지 합본)" },
  { mode: "indexVsHead", label: "스테이지됨", title: "인덱스 ↔ HEAD(git add 된 변경)" },
  { mode: "worktreeVsIndex", label: "미스테이지", title: "워킹트리 ↔ 인덱스(아직 add 안 된 변경)" },
];

/** 경로의 마지막 세그먼트(파일명). */
function basename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

/** diff 본문(로딩/바이너리/잘림/빈 상태 처리 공통). */
function DiffBody({ diff, loading }: { diff?: GitDiffResult; loading: boolean }) {
  if (loading && !diff) return <div className="wd-detail-empty">변경점 불러오는 중…</div>;
  if (!diff) return <div className="wd-detail-empty">변경점을 선택하세요.</div>;
  if (diff.timedOut)
    return <div className="wd-detail-empty">조회가 시간 초과됐습니다. 다시 시도하세요.</div>;
  if (diff.binary) return <div className="wd-detail-empty">바이너리 파일이라 diff를 표시할 수 없습니다.</div>;
  if (diff.diff.trim() === "") return <div className="wd-detail-empty">표시할 변경이 없습니다.</div>;
  return (
    <>
      {diff.truncated && (
        <div className="wd-note">변경이 커서 일부(최대 5000줄)만 표시됩니다.</div>
      )}
      <DiffView diff={diff.diff} />
    </>
  );
}

/** git 상태 뱃지 span(파일 목록·커밋 파일 공통 모양). */
function StatusBadge({ status }: { status: string }) {
  return (
    <span
      className={`wd-badge wd-badge-${status}`}
      title={statusLabel(status)}
      aria-label={statusLabel(status)}
    >
      {status}
    </span>
  );
}

export function WorkdirDetailPane() {
  const detail = useWorkdirStore((s) => s.detail);
  const closeDetail = useWorkdirStore((s) => s.closeDetail);
  const setDetailTab = useWorkdirStore((s) => s.setDetailTab);
  const setDiffMode = useWorkdirStore((s) => s.setDiffMode);
  const selectCommit = useWorkdirStore((s) => s.selectCommit);
  const toggleCommitExpand = useWorkdirStore((s) => s.toggleCommitExpand);
  const loadMoreCommitFiles = useWorkdirStore((s) => s.loadMoreCommitFiles);
  const selectCommitFile = useWorkdirStore((s) => s.selectCommitFile);
  const openExternal = useWorkdirStore((s) => s.openExternal);
  const openInApp = useWorkdirStore((s) => s.openInApp);
  const openDifftool = useWorkdirStore((s) => s.openDifftool);

  if (!detail) return null;

  const canInApp = isMarkdownPath(detail.relPath);
  // 하단 diff가 이 상세 파일이 아닌 다른(펼친 커밋의) 파일을 보고 있으면 라벨 표시.
  const diffFileLabel =
    detail.selectedCommitFile && detail.selectedCommitFile !== detail.relPath
      ? basename(detail.selectedCommitFile)
      : null;

  return (
    <div className="wd-detail" role="region" aria-label="파일 메뉴·변경점·히스토리">
      <div className="wd-detail-head">
        <div className="wd-detail-title" title={detail.relPath}>
          {detail.status && <StatusBadge status={detail.status} />}
          {detail.name}
          <span className="wd-detail-path">{detail.relPath}</span>
        </div>
        <button type="button" className="wd-close" aria-label="상세 닫기" onClick={closeDetail}>
          ×
        </button>
      </div>

      <div className="wd-detail-actions">
        <button
          type="button"
          className="wd-btn"
          title="OS 기본/VS Code 등 외부 프로그램으로 엽니다"
          onClick={() => openExternal()}
        >
          외부 프로그램으로 열기
        </button>
        {canInApp && (
          <button
            type="button"
            className="wd-btn"
            title="인앱 마크다운 뷰어/편집기로 엽니다"
            onClick={() => openInApp()}
          >
            인앱 뷰어로 열기
          </button>
        )}
      </div>

      <div className="wd-tabs" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={detail.tab === "diff"}
          className={detail.tab === "diff" ? "wd-tab wd-tab-active" : "wd-tab"}
          onClick={() => setDetailTab("diff")}
        >
          변경점
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={detail.tab === "history"}
          className={detail.tab === "history" ? "wd-tab wd-tab-active" : "wd-tab"}
          onClick={() => setDetailTab("history")}
        >
          히스토리
        </button>
      </div>

      {detail.tab === "diff" ? (
        <div className="wd-detail-body">
          {!detail.isUntracked && (
            <div className="wd-seg wd-mode-seg" role="group" aria-label="diff 관점">
              {MODE_TABS.map((t) => (
                <button
                  key={t.mode}
                  type="button"
                  title={t.title}
                  className={
                    detail.diffMode === t.mode ? "wd-seg-btn wd-seg-active" : "wd-seg-btn"
                  }
                  onClick={() => setDiffMode(t.mode)}
                >
                  {t.label}
                </button>
              ))}
            </div>
          )}
          <DiffBody diff={detail.diff} loading={detail.diffLoading} />
        </div>
      ) : (
        <div className="wd-detail-body">
          {detail.historyLoading && !detail.history ? (
            <div className="wd-detail-empty">히스토리 불러오는 중…</div>
          ) : !detail.history || detail.history.length === 0 ? (
            <div className="wd-detail-empty">커밋 히스토리가 없습니다.</div>
          ) : (
            <>
              <ul className="wd-history" role="listbox" aria-label="커밋 목록">
                {detail.history.map((c) => {
                  const expanded = detail.expandedCommit === c.hash;
                  const active = detail.selectedCommit === c.hash;
                  return (
                    <li
                      key={c.hash}
                      role="option"
                      aria-selected={active}
                      className={expanded || active ? "wd-commit wd-commit-active" : "wd-commit"}
                    >
                      <div className="wd-commit-row">
                        <button
                          type="button"
                          className="wd-commit-toggle"
                          aria-expanded={expanded}
                          title={expanded ? "변경파일 접기" : "이 커밋이 바꾼 파일 보기"}
                          onClick={() => toggleCommitExpand(c.hash)}
                        >
                          {expanded ? "▾" : "▸"}
                        </button>
                        <div className="wd-commit-main" onClick={() => selectCommit(c.hash)}>
                          <span className="wd-commit-hash">{c.shortHash}</span>
                          <span className="wd-commit-subject">{c.subject}</span>
                          <span className="wd-commit-meta">
                            {c.author} · {c.date}
                          </span>
                        </div>
                      </div>
                      {expanded && (
                        <ul className="wd-commit-files" aria-label="이 커밋이 바꾼 파일">
                          {detail.commitFilesLoading && !detail.commitFiles ? (
                            <li className="wd-cf-note">변경파일 불러오는 중…</li>
                          ) : (detail.commitFiles ?? []).length === 0 ? (
                            <li className="wd-cf-note">
                              표시할 파일 변경이 없습니다(병합 커밋일 수 있음).
                            </li>
                          ) : (
                            <>
                              {(detail.commitFiles ?? []).map((f) => (
                                <li
                                  key={f.path}
                                  className={
                                    detail.selectedCommit === c.hash &&
                                    detail.selectedCommitFile === f.path
                                      ? "wd-cf wd-cf-active"
                                      : "wd-cf"
                                  }
                                  title={f.path}
                                  onClick={() => selectCommitFile(c.hash, f.path)}
                                >
                                  <StatusBadge status={f.status} />
                                  <span className="wd-cf-name">{basename(f.path)}</span>
                                  <span className="wd-cf-path">{f.path}</span>
                                </li>
                              ))}
                              {detail.commitFilesHasMore && (
                                <li
                                  className="wd-cf-more"
                                  onClick={() => loadMoreCommitFiles()}
                                >
                                  {detail.commitFilesLoading ? "불러오는 중…" : "더 보기…"}
                                </li>
                              )}
                            </>
                          )}
                        </ul>
                      )}
                    </li>
                  );
                })}
              </ul>
              {detail.historyHasMore && (
                <div className="wd-note">최근 {detail.history.length}개만 표시됩니다.</div>
              )}
              {detail.selectedCommit && (
                <div className="wd-commit-diff">
                  <div className="wd-detail-actions">
                    {diffFileLabel && (
                      <span className="wd-cf-difflabel" title={detail.selectedCommitFile}>
                        {diffFileLabel}
                      </span>
                    )}
                    <button
                      type="button"
                      className="wd-btn"
                      title="이 커밋의 변경을 외부 비교 도구로 엽니다"
                      onClick={() => openDifftool(detail.selectedCommit)}
                    >
                      외부 도구로 비교
                    </button>
                  </div>
                  <DiffBody diff={detail.commitDiff} loading={detail.commitDiffLoading} />
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
