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
//
// 헤더 브랜치 요약 옆에 파일 목록 캐시의 기준 시각("N분 전 기준")을 보여주고,
// 새로고침 버튼으로 TTL과 무관하게 목록·git 상태를 강제 재조회한다(이슈 #67).
//
// 서버사이드 검색(이슈 #67 후속): Everything 백엔드 + "전체" 필터 + 파일 목록
// 뷰에서 쿼리가 2글자 이상이면 workdirStore.setQuery가 디바운스 후 es.exe로
// 다시 검색한다(listing의 5000개 상한 밖 파일도 찾기 위함). 활성 검색 결과가
// 있으면(searchActive) 그 결과를 rank-only(fuzzyRank, 탈락 없음)로 보여주고,
// 아니면 기존처럼 이미 가져온 목록 안에서 fuzzyFilter(탈락 있음)로 거른다.
//
// 미추적 파일은 백엔드가 `-uall`로 파일 단위까지 펼쳐 주므로(이슈 #70) 새로
// 추가된 폴더 안의 파일도 각각 '?' 뱃지로 뜬다 — 5000개 상한에 걸리면
// `git.truncated`로 안내한다. 행의 상대경로는 앞쪽이 말줄임되고(이슈 #71,
// workdir.css) 전체 경로는 행 `title` 툴팁으로 확인한다.
import { useEffect, useMemo, useRef } from "react";
import { useWorkdirStore } from "./workdirStore";
import { WorkdirDetailPane } from "./WorkdirDetailPane";
import { WorkdirRepoLogPane } from "./WorkdirRepoLogPane";
import { statusLabel } from "./status";
import { useAppStore } from "../store/appStore";
import { fuzzyFilter, fuzzyRank } from "../markdown/fuzzy";
import { formatRelativeTime } from "../shared/relativeTime";
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

export function WorkdirPalette() {
  const palette = useWorkdirStore((s) => s.palette);
  const listing = useWorkdirStore((s) => (s.palette ? s.listing[s.palette.root] : undefined));
  const search = useWorkdirStore((s) => s.search);
  const searchLoading = useWorkdirStore((s) => s.searchLoading);
  const git = useWorkdirStore((s) => (s.palette ? s.git[s.palette.root] : undefined));
  const gitLoading = useWorkdirStore((s) => (s.palette ? !!s.gitLoading[s.palette.root] : false));
  const detailOpen = useWorkdirStore((s) => s.detail !== null);
  const setQuery = useWorkdirStore((s) => s.setQuery);
  const setSelectedIndex = useWorkdirStore((s) => s.setSelectedIndex);
  const setChangedOnly = useWorkdirStore((s) => s.setChangedOnly);
  const viewMode = useWorkdirStore((s) => s.palette?.viewMode ?? "files");
  const setViewMode = useWorkdirStore((s) => s.setViewMode);
  const closePalette = useWorkdirStore((s) => s.closePalette);
  const closeDetail = useWorkdirStore((s) => s.closeDetail);
  const openEntry = useWorkdirStore((s) => s.openEntry);
  const openDetail = useWorkdirStore((s) => s.openDetail);
  const refreshGit = useWorkdirStore((s) => s.refreshGit);
  const refreshListing = useWorkdirStore((s) => s.refreshListing);

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

  // 서버사이드(Everything) 검색이 활성인지(이슈 #67): 현재 root·query와
  // 정확히 일치하고, "전체" 필터(changedOnly 아님)·파일 목록 뷰일 때만 우선한다
  // (changedOnly/log 뷰는 애초에 setQuery가 search를 채우지 않지만, 응답이
  // 도착한 뒤 사용자가 필터를 바꿨을 수 있어 여기서도 다시 확인한다).
  const searchActive =
    !changedOnly &&
    viewMode === "files" &&
    !!search &&
    search.root === root &&
    search.query === query &&
    search.files.length > 0;

  // 서버 검색 결과에도 기존 목록과 동일하게 gitMap 뱃지를 얹는다.
  const searchRows: RowItem[] = useMemo(() => {
    if (!searchActive || !search) return [];
    return search.files.map((f) => {
      const g = gitMap.get(f.relPath);
      return { relPath: f.relPath, name: f.name, status: g?.status, xy: g?.xy };
    });
  }, [searchActive, search, gitMap]);

  // 퍼지 필터(원본 참조 안정 시에만 재계산). 서버 검색이 활성이면 서버가 이미
  // 후보를 좁혀 줬으므로 탈락 없는 rank-only 정렬(fuzzyRank), 아니면 기존
  // 클라이언트 필터(fuzzyFilter).
  const results = useMemo(
    () =>
      searchActive
        ? fuzzyRank(searchRows, query).map((r) => r.item)
        : fuzzyFilter(rows, query).map((r) => r.item),
    [searchActive, searchRows, rows, query],
  );

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

  // 이슈 #54: 모든 파일 클릭은 곧장 열지 않고 상세(메뉴) 페인을 띄운다 — 거기서
  // 깃 로그·외부/인앱 열기를 고른다. 빠른 열기는 ⌘-클릭/더블클릭으로 기존 자동
  // 라우팅(openEntry)을 그대로 쓴다.
  const openMenu = (index: number) => {
    const item = results[index];
    if (!item) return;
    openDetail(root, item.relPath, item.name, item.status);
  };
  const openImmediate = (index: number) => {
    const item = results[index];
    if (!item) return;
    openEntry(root, item.relPath, item.name);
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
      // ⌘/Ctrl+Enter는 즉시 열기, 그냥 Enter는 메뉴.
      if (e.metaKey || e.ctrlKey) openImmediate(selected);
      else openMenu(selected);
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

  // 새로고침 버튼(이슈 #67): TTL을 무시하고 목록·git 상태를 강제로 다시 조회.
  const onRefresh = () => {
    void refreshListing(root, { force: true });
    void refreshGit(root);
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
    // 상한(5000)에 걸렸으면 "+"를 붙여 더 있음을 알린다(이슈 #70).
    parts.push(`· 변경 ${git.entries.length}${git.truncated ? "+" : ""}개`);
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
        className={detailOpen || viewMode === "log" ? "wd-panel wd-panel-wide" : "wd-panel"}
        role="dialog"
        aria-label="작업 폴더 보기"
      >
        <div className="wd-header">
          <div className="wd-branch" title={root}>
            {branchSummary}
            {listing && (
              <span className="wd-updated"> · {formatRelativeTime(listing.fetchedAt)} 기준</span>
            )}
          </div>
          <div className="wd-header-actions">
            <button
              type="button"
              className="wd-refresh"
              title="새로고침"
              onClick={onRefresh}
            >
              ↻
            </button>
            {/* 파일 목록 ↔ 저장소 전체 커밋 로그 브라우저(이슈 #54). */}
            <div className="wd-seg" role="group" aria-label="뷰">
              <button
                type="button"
                className={viewMode === "files" ? "wd-seg-btn wd-seg-active" : "wd-seg-btn"}
                onClick={() => setViewMode("files")}
              >
                파일
              </button>
              <button
                type="button"
                className={viewMode === "log" ? "wd-seg-btn wd-seg-active" : "wd-seg-btn"}
                disabled={!hasGit}
                title={hasGit ? "" : "git 저장소여야 커밋 로그를 볼 수 있습니다"}
                onClick={() => setViewMode("log")}
              >
                커밋 로그
              </button>
            </div>
            {viewMode === "files" && (
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
            )}
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
        {viewMode === "log" ? (
          <WorkdirRepoLogPane />
        ) : (
        <>
        <input
          ref={inputRef}
          className="wd-input"
          type="text"
          placeholder="파일 이름 또는 경로로 검색…  (⌘/더블클릭: 즉시 열기)"
          value={query}
          spellCheck={false}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
        />
        <div className="wd-body">
          <div className="wd-list-pane">
            {searchActive && search?.truncated ? (
              <div className="wd-note">일치 항목이 많아 일부(5000개)만 표시됩니다.</div>
            ) : (
              listing?.truncated &&
              !changedOnly && <div className="wd-note">파일이 많아 일부(5000개)만 표시됩니다.</div>
            )}
            {searchLoading && <div className="wd-note wd-note-dim">검색 중…</div>}
            {git?.timedOut && (
              <div className="wd-note">
                git 상태 조회가 시간 초과됐습니다. 설정에서 끌 수 있습니다.
              </div>
            )}
            {git?.truncated && (
              <div className="wd-note">변경된 파일이 많아 일부(5000개)만 표시됩니다.</div>
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
                    // 경로가 잘려도 호버로 전체를 확인할 수 있게(이슈 #71).
                    title={item.relPath}
                    className={i === selected ? "wd-item wd-item-active" : "wd-item"}
                    onMouseDown={(e) => {
                      // 포커스를 입력창에 유지하고 선택만 옮긴다(열기는 click에서).
                      e.preventDefault();
                      setSelectedIndex(i);
                    }}
                    onClick={(e) => {
                      // ⌘/Ctrl-클릭은 즉시 열기, 그냥 클릭은 메뉴 페인.
                      if (e.metaKey || e.ctrlKey) openImmediate(i);
                      else openMenu(i);
                    }}
                    onDoubleClick={() => openImmediate(i)}
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
        </>
        )}
      </div>
    </div>
  );
}
