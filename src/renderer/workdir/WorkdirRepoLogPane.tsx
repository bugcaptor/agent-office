// src/renderer/workdir/WorkdirRepoLogPane.tsx
//
// 저장소 전체 커밋 로그 브라우저(이슈 #54, 2단계). 파일을 먼저 지목하지 않고
// 로그 → 커밋 → 변경파일 → diff 순으로 훑는다. 좌측 커밋 목록(검색·전체브랜치·
// 더 보기), 우측은 선택 커밋의 변경파일 목록과 고른 파일의 그 커밋 diff.
//
// 검색 입력은 매 타건마다 git을 때리지 않도록 300ms 디바운스로 스토어 쿼리에
// 반영한다(스토어가 쿼리 변경 시 첫 페이지부터 재조회).
import { useEffect, useRef, useState } from "react";
import { useWorkdirStore } from "./workdirStore";
import { DiffView } from "./DiffView";
import { statusLabel } from "./status";
import type { GitDiffResult } from "@shared/types";

/** 경로의 마지막 세그먼트(파일명). */
function basename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

/** diff 본문(상세 페인과 동일한 상태 처리). */
function DiffBody({ diff, loading }: { diff?: GitDiffResult; loading: boolean }) {
  if (loading && !diff) return <div className="wd-detail-empty">변경점 불러오는 중…</div>;
  if (!diff) return <div className="wd-detail-empty">파일을 선택하세요.</div>;
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

export function WorkdirRepoLogPane() {
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
          placeholder="커밋 메시지 검색…"
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
        <label className="wd-git-toggle" title="모든 브랜치/참조의 커밋을 함께 봅니다">
          <input
            type="checkbox"
            checked={rl?.allBranches ?? false}
            onChange={(e) => setRepoLogAllBranches(e.target.checked)}
          />
          <span>전체 브랜치</span>
        </label>
      </div>

      <div className="wd-log-body">
        {/* 좌: 커밋 목록 */}
        <div className="wd-log-commits">
          {rl?.timedOut && (
            <div className="wd-note">로그 조회가 시간 초과됐습니다. 검색을 좁혀 보세요.</div>
          )}
          {commits === undefined ? (
            <div className="wd-empty">로그를 불러오는 중…</div>
          ) : commits.length === 0 ? (
            <div className="wd-empty">
              {appliedQuery ? "검색과 일치하는 커밋이 없습니다." : "커밋이 없습니다."}
            </div>
          ) : (
            <ul className="wd-history" role="listbox" aria-label="커밋 목록">
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
                  {rl?.loading ? "불러오는 중…" : "더 보기…"}
                </li>
              )}
            </ul>
          )}
        </div>

        {/* 우: 선택 커밋의 변경파일 + 고른 파일 diff */}
        <div className="wd-log-detail">
          {!selectedCommit ? (
            <div className="wd-detail-empty">커밋을 선택하면 바뀐 파일이 나옵니다.</div>
          ) : (
            <>
              <ul className="wd-commit-files wd-log-files" aria-label="이 커밋이 바꾼 파일">
                {rl?.filesLoading && !rl?.files ? (
                  <li className="wd-cf-note">변경파일 불러오는 중…</li>
                ) : (rl?.files ?? []).length === 0 ? (
                  <li className="wd-cf-note">
                    표시할 파일 변경이 없습니다(병합 커밋일 수 있음).
                  </li>
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
                          title={statusLabel(f.status)}
                          aria-label={statusLabel(f.status)}
                        >
                          {f.status}
                        </span>
                        <span className="wd-cf-name">{basename(f.path)}</span>
                        <span className="wd-cf-path">{f.path}</span>
                      </li>
                    ))}
                    {rl?.filesHasMore && (
                      <li className="wd-cf-more" onClick={() => loadMoreRepoFiles()}>
                        {rl?.filesLoading ? "불러오는 중…" : "더 보기…"}
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
                      title="이 커밋의 변경을 외부 비교 도구로 엽니다"
                      onClick={() => openRepoDifftool()}
                    >
                      외부 도구로 비교
                    </button>
                  </div>
                  <DiffBody diff={rl.fileDiff} loading={rl.fileDiffLoading} />
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
