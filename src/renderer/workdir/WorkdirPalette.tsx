// src/renderer/workdir/WorkdirPalette.tsx
//
// 작업 폴더 보기 오버레이(이슈 #11). 에이전트 cwd를 root로 파일 목록을 보여주고,
// 파일별 git 상태를 단일 문자 뱃지로 얹는다. 헤더에 브랜치 요약과 [전체|변경만]
// 필터, git 상태 on/off 토글을 둔다(토글은 전역 설정과 같은 값 — 상태 이원화 방지).
//
// self-gate 관례(MarkdownPalette와 동일): 항상 마운트되며 팔레트가 없으면 null.
// 키 이벤트는 여기서 stopPropagation해 터미널/전역 단축키로 새지 않게 한다.
//
// "전체" 뷰는 파일 목록(.gitignore 존중)에 git 상태를 relPath로 매칭해 뱃지를
// 얹고, "변경만" 뷰는 git 엔트리 자체를 목록으로 쓴다(삭제·root 밖 "../" 파일도
// 포함). git이 꺼졌거나 저장소가 아니면 "변경만"은 비활성.
import { useEffect, useMemo, useRef } from "react";
import { useWorkdirStore, isChangedStatus } from "./workdirStore";
import { WorkdirDetailPane } from "./WorkdirDetailPane";
import { useAppStore } from "../store/appStore";
import { fuzzyFilter } from "../markdown/fuzzy";
import type { GitFileStatus } from "@shared/types";

/** 목록 행 하나(전체/변경만 공통). status/xy는 git 뱃지용(없을 수 있음). */
interface RowItem {
  relPath: string;
  name: string;
  status?: string;
  xy?: string;
}

/** 경로의 마지막 세그먼트(파일명). */
function basename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

/** 뱃지 문자 → 사람이 읽는 상태(툴팁·접근성). */
function statusLabel(status: string): string {
  switch (status) {
    case "M":
      return "수정됨";
    case "A":
      return "추가됨";
    case "D":
      return "삭제됨";
    case "R":
      return "이름변경";
    case "C":
      return "복사됨";
    case "U":
      return "충돌";
    case "T":
      return "타입변경";
    case "?":
      return "추적 안 됨";
    default:
      return status;
  }
}

export function WorkdirPalette() {
  const palette = useWorkdirStore((s) => s.palette);
  const listing = useWorkdirStore((s) => (s.palette ? s.listing[s.palette.root] : undefined));
  const git = useWorkdirStore((s) => (s.palette ? s.git[s.palette.root] : undefined));
  const gitLoading = useWorkdirStore((s) => (s.palette ? !!s.gitLoading[s.palette.root] : false));
  const detailOpen = useWorkdirStore((s) => s.detail !== null);
  const setQuery = useWorkdirStore((s) => s.setQuery);
  const setSelectedIndex = useWorkdirStore((s) => s.setSelectedIndex);
  const setChangedOnly = useWorkdirStore((s) => s.setChangedOnly);
  const closePalette = useWorkdirStore((s) => s.closePalette);
  const closeDetail = useWorkdirStore((s) => s.closeDetail);
  const openEntry = useWorkdirStore((s) => s.openEntry);
  const openDetail = useWorkdirStore((s) => s.openDetail);
  const refreshGit = useWorkdirStore((s) => s.refreshGit);

  const gitStatusEnabled = useAppStore((s) => s.appSettings.gitStatusEnabled);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const query = palette?.query ?? "";
  const changedOnly = palette?.changedOnly ?? false;
  const root = palette?.root ?? "";

  // relPath → git 상태 맵(뱃지 매칭용).
  const gitMap = useMemo(() => {
    const m = new Map<string, GitFileStatus>();
    if (git?.isRepo) for (const e of git.entries) m.set(e.path, e);
    return m;
  }, [git]);

  const hasGit = !!git?.isRepo && !git.timedOut;

  // 표시 대상 행 목록(전체 vs 변경만).
  const rows: RowItem[] = useMemo(() => {
    if (changedOnly && hasGit) {
      return (git?.entries ?? []).map((e) => ({
        relPath: e.path,
        name: basename(e.path),
        status: e.status,
        xy: e.xy,
      }));
    }
    return (listing?.files ?? []).map((f) => {
      const g = gitMap.get(f.relPath);
      return { relPath: f.relPath, name: f.name, status: g?.status, xy: g?.xy };
    });
  }, [changedOnly, hasGit, git, listing, gitMap]);

  // 퍼지 필터(원본 참조 안정 시에만 재계산).
  const results = useMemo(() => fuzzyFilter(rows, query).map((r) => r.item), [rows, query]);

  const selected = Math.min(
    Math.max(palette?.selectedIndex ?? 0, 0),
    Math.max(results.length - 1, 0),
  );

  const open = palette !== null;
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  useEffect(() => {
    const el = listRef.current?.children[selected] as HTMLElement | undefined;
    if (el && typeof el.scrollIntoView === "function") el.scrollIntoView({ block: "nearest" });
  }, [selected, results.length]);

  if (!palette) return null;

  // 변경된 파일(git 뱃지 있음)은 곧장 열지 않고 상세(변경점) 페인으로 보낸다.
  // 변경 없는 파일만 기존처럼 바로 연다(openEntry).
  const commitOpen = (index: number) => {
    const item = results[index];
    if (!item) return;
    if (isChangedStatus(item.status)) openDetail(root, item.relPath, item.name, item.status);
    else openEntry(root, item.relPath, item.name);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    e.stopPropagation();
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex(Math.min(selected + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex(Math.max(selected - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      commitOpen(selected);
    } else if (e.key === "Escape") {
      e.preventDefault();
      // 상세가 열려 있으면 상세만 먼저 닫는다.
      if (detailOpen) closeDetail();
      else closePalette();
    }
  };

  // git 토글: 전역 설정과 동일 값. 켤 때는 즉시 재조회.
  const toggleGit = () => {
    const next = !gitStatusEnabled;
    updateAppSettings({ gitStatusEnabled: next });
    if (next) void refreshGit(root);
    else if (changedOnly) setChangedOnly(false); // 데이터가 사라지므로 전체로 복귀.
  };

  // 브랜치 요약 문자열.
  const branchSummary = (() => {
    if (!gitStatusEnabled) return "git 상태 꺼짐";
    if (gitLoading && !git) return "git 상태 조회 중…";
    if (git?.timedOut) return "git 상태 조회 시간 초과";
    if (!git?.isRepo) return "git 저장소 아님";
    const parts: string[] = [git.branch ?? "(detached)"];
    if (git.ahead) parts.push(`↑${git.ahead}`);
    if (git.behind) parts.push(`↓${git.behind}`);
    parts.push(`· 변경 ${git.entries.length}개`);
    return parts.join(" ");
  })();

  return (
    <div
      className="wd-overlay"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closePalette();
      }}
    >
      <div
        className={detailOpen ? "wd-panel wd-panel-wide" : "wd-panel"}
        role="dialog"
        aria-label="작업 폴더 보기"
      >
        <div className="wd-header">
          <div className="wd-branch" title={root}>
            {branchSummary}
          </div>
          <div className="wd-header-actions">
            <div className="wd-seg" role="group" aria-label="필터">
              <button
                type="button"
                className={changedOnly ? "wd-seg-btn" : "wd-seg-btn wd-seg-active"}
                onClick={() => setChangedOnly(false)}
              >
                전체
              </button>
              <button
                type="button"
                className={changedOnly ? "wd-seg-btn wd-seg-active" : "wd-seg-btn"}
                disabled={!hasGit}
                title={hasGit ? "" : "git 상태가 있어야 사용할 수 있습니다"}
                onClick={() => setChangedOnly(true)}
              >
                변경만
              </button>
            </div>
            <label className="wd-git-toggle" title="파일별 git 상태 조회(거대 저장소에서 끄기)">
              <input type="checkbox" checked={gitStatusEnabled} onChange={toggleGit} />
              <span>git 상태</span>
            </label>
            <button
              type="button"
              className="wd-close"
              aria-label="닫기"
              onClick={closePalette}
            >
              ×
            </button>
          </div>
        </div>
        <input
          ref={inputRef}
          className="wd-input"
          type="text"
          placeholder="파일 이름 또는 경로로 검색…"
          value={query}
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
        />
        <div className="wd-body">
          <div className="wd-list-pane">
            {listing?.truncated && !changedOnly && (
              <div className="wd-note">파일이 많아 일부(5000개)만 표시됩니다.</div>
            )}
            {git?.timedOut && (
              <div className="wd-note">
                git 상태 조회가 시간 초과됐습니다. 설정에서 끌 수 있습니다.
              </div>
            )}
            {listing === undefined && !changedOnly ? (
              <div className="wd-empty">목록을 불러오는 중…</div>
            ) : results.length === 0 ? (
              <div className="wd-empty">
                {changedOnly
                  ? "변경된 파일이 없습니다."
                  : rows.length === 0
                    ? "파일이 없습니다."
                    : "일치하는 파일이 없습니다."}
              </div>
            ) : (
              <ul className="wd-list" ref={listRef} role="listbox">
                {results.map((item, i) => (
                  <li
                    key={item.relPath}
                    role="option"
                    aria-selected={i === selected}
                    className={i === selected ? "wd-item wd-item-active" : "wd-item"}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      setSelectedIndex(i);
                      commitOpen(i);
                    }}
                    onMouseEnter={() => setSelectedIndex(i)}
                  >
                    <span
                      className={
                        item.status
                          ? `wd-badge wd-badge-${item.status}`
                          : "wd-badge wd-badge-empty"
                      }
                      title={item.status ? `${statusLabel(item.status)} (${item.xy})` : ""}
                      aria-label={item.status ? statusLabel(item.status) : undefined}
                    >
                      {item.status ?? ""}
                    </span>
                    <span className="wd-item-name">{item.name}</span>
                    <span className="wd-item-path">{item.relPath}</span>
                  </li>
                ))}
              </ul>
            )}
          </div>
          {detailOpen && <WorkdirDetailPane />}
        </div>
      </div>
    </div>
  );
}
