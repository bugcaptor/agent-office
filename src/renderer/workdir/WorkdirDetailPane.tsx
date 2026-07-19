// src/renderer/workdir/WorkdirDetailPane.tsx
//
// 작업 폴더 보기(이슈 #11 후속)의 우측 상세 페인. 변경된 파일을 클릭하면 곧장
// 열지 않고 여기서 **변경점(diff)** 을 먼저 보여준다. 두 탭:
//   - 변경점: 워킹트리↔HEAD(전체)·인덱스↔HEAD(스테이지됨)·워킹트리↔인덱스
//     (미스테이지) 세 관점 전환(미추적 파일은 단일 뷰). "실제 파일 열기"·
//     "외부 도구로 비교" 버튼.
//   - 히스토리: `git log --follow` 커밋 목록 → 커밋 선택 시 그 커밋의 diff.
import { useWorkdirStore } from "./workdirStore";
import { DiffView } from "./DiffView";
import type { GitDiffMode, GitDiffResult } from "@shared/types";

/** diff 모드 → 탭 라벨(추적 파일용 3탭). */
const MODE_TABS: { mode: GitDiffMode; label: string; title: string }[] = [
  { mode: "worktreeVsHead", label: "전체", title: "워킹트리 ↔ HEAD(스테이지+미스테이지 합본)" },
  { mode: "indexVsHead", label: "스테이지됨", title: "인덱스 ↔ HEAD(git add 된 변경)" },
  { mode: "worktreeVsIndex", label: "미스테이지", title: "워킹트리 ↔ 인덱스(아직 add 안 된 변경)" },
];

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

export function WorkdirDetailPane() {
  const detail = useWorkdirStore((s) => s.detail);
  const closeDetail = useWorkdirStore((s) => s.closeDetail);
  const setDetailTab = useWorkdirStore((s) => s.setDetailTab);
  const setDiffMode = useWorkdirStore((s) => s.setDiffMode);
  const selectCommit = useWorkdirStore((s) => s.selectCommit);
  const openEntry = useWorkdirStore((s) => s.openEntry);
  const openDifftool = useWorkdirStore((s) => s.openDifftool);

  if (!detail) return null;

  const openActual = () => openEntry(detail.root, detail.relPath, detail.name);

  return (
    <div className="wd-detail" role="region" aria-label="변경점·히스토리">
      <div className="wd-detail-head">
        <div className="wd-detail-title" title={detail.relPath}>
          {detail.name}
          <span className="wd-detail-path">{detail.relPath}</span>
        </div>
        <button type="button" className="wd-close" aria-label="상세 닫기" onClick={closeDetail}>
          ×
        </button>
      </div>

      <div className="wd-detail-actions">
        <button type="button" className="wd-btn" onClick={openActual}>
          실제 파일 열기
        </button>
        {!detail.isUntracked && (
          <button
            type="button"
            className="wd-btn"
            title="git difftool로 외부 비교 도구를 띄웁니다(설정돼 있어야 함)"
            onClick={() => openDifftool()}
          >
            외부 도구로 비교
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
                {detail.history.map((c) => (
                  <li
                    key={c.hash}
                    role="option"
                    aria-selected={detail.selectedCommit === c.hash}
                    className={
                      detail.selectedCommit === c.hash
                        ? "wd-commit wd-commit-active"
                        : "wd-commit"
                    }
                    onClick={() => selectCommit(c.hash)}
                  >
                    <span className="wd-commit-hash">{c.shortHash}</span>
                    <span className="wd-commit-subject">{c.subject}</span>
                    <span className="wd-commit-meta">
                      {c.author} · {c.date}
                    </span>
                  </li>
                ))}
              </ul>
              {detail.historyHasMore && (
                <div className="wd-note">최근 {detail.history.length}개만 표시됩니다.</div>
              )}
              {detail.selectedCommit && (
                <div className="wd-commit-diff">
                  <div className="wd-detail-actions">
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
