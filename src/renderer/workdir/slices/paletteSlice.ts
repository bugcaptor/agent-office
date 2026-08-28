// src/renderer/workdir/slices/paletteSlice.ts
//
// 팔레트 자체(열기·닫기·쿼리·선택·뷰 모드)와 그 재료가 되는 두 캐시:
// 파일 목록(listing)과 git 상태(git). 서버사이드 검색(이슈 #67)도 쿼리에
// 딸린 관심사라 여기 있다.
//
// 불변식:
// - 목록·git 캐시는 root별로 유지되어 재오픈 시 즉시 표시되고 백그라운드 갱신된다.
// - git 상태는 앱 설정 `gitStatusEnabled`가 켜졌을 때만 조회한다(거대 저장소
//   대비 off 스위치).
// - git status 경로는 git을 실행한 cwd(=팔레트 root) 기준이라 파일 목록의
//   relPath와 그대로 매칭된다.
//
// 파일 열기(openEntry, ⌘-클릭/더블클릭 빠른 열기): .md는 인앱 편집기
// (markdownStore.openFile)로 위임하고, 그 외는 절대경로를 만들어
// open_in_vscode로 외부 에디터에 넘긴다. 그냥 클릭은 열지 않고 상세 페인을
// 띄운다(detailSlice).
//
// 서버사이드 검색(이슈 #67): 목록(listing)은 5000개 상한에 걸려 잘릴 수 있어,
// 그 밖의 파일은 클라이언트 fuzzy 필터로 찾을 수 없다. 설정 `fileIndexBackend`가
// Everything이면 `setQuery`가 (디바운스 후) es.exe로 다시 검색해 `search`를
// 채우고, 팔레트는 활성 `search`가 있으면 그 결과를 우선 보여준다
// (WorkdirPalette.tsx의 `results` 분기). Walker 백엔드/짧은 쿼리/변경만 필터/
// 로그 뷰/es.exe 실패는 모두 `search: null`로 기존 클라이언트 필터 경로를 쓴다.
import { tauriApi } from "../../ipc/tauriApi";
import { useAppStore } from "../../store/appStore";
import { useMarkdownStore } from "../../markdown/markdownStore";
import { createInFlightTracker, isStale } from "../../shared/createListingCache";

import { cancel, cancelDetailOps, cancelRepoLogOps, newOpId, omitKey } from "../workdirOps";
import { isMarkdownPath, joinPath } from "../workdirTypes";
import type { WorkdirPaletteSlice, WorkdirSlice } from "../workdirTypes";

/** 캐시 재사용 유효기간(이슈 #67): 이보다 오래되면 팔레트를 열 때 백그라운드로
 *  재스캔한다. 그 이내면 캐시를 그대로 쓰고 풀스캔을 건너뛴다. */
const LISTING_TTL_MS = 5 * 60_000;

/** root별 refreshListing 진행 상태(모듈 수준 — 스토어 재생성과 무관하게 유지). */
const listingInFlight = createInFlightTracker();

/** 서버사이드(Everything) 검색 디바운스 지연(ms, 이슈 #67) — 타이핑 중 매
 *  keystroke마다 es.exe를 부르지 않기 위함. */
const SEARCH_DEBOUNCE_MS = 250;
/** 서버사이드 검색을 시도할 최소 쿼리 길이. 한두 글자는 후보가 너무 많아
 *  Everything 조회 자체가 무의미하다(클라이언트 fuzzy 필터로 충분). */
const SEARCH_MIN_QUERY_LEN = 2;

/** 팔레트 검색어 디바운스 타이머(모듈 수준 — 스토어 재생성과 무관하게 유지,
 *  root/query가 바뀌면 이전 타이머를 취소한다). */
let searchDebounceTimer: ReturnType<typeof setTimeout> | undefined;
/** 서버사이드 검색 요청 세대 카운터. 응답 도착 시 이 값과 다르면(그 사이 더
 *  최신 요청이 나갔으면) stale로 보고 폐기한다. */
let searchGen = 0;

export const createPaletteSlice: WorkdirSlice<WorkdirPaletteSlice> = (set, get) => ({
  palette: null,
  listing: {},
  search: null,
  searchLoading: false,
  git: {},
  gitLoading: {},
  gitOpId: {},

  openPalette: (root, agentId) => {
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    searchGen++;
    // 이전 팔레트의 상세가 걸어 둔 조회는 여기서 무의미해진다.
    cancelDetailOps(get().detail);
    set({
      palette: { root, agentId, query: "", selectedIndex: 0, changedOnly: false, viewMode: "files" },
      detail: null,
      search: null,
      searchLoading: false,
    });
    // 캐시가 있으면 즉시 표시된다. 목록 재스캔 여부(TTL)는 refreshListing
    // 내부가 판단하고, git 상태는 캐시 없이 매번 갱신한다.
    void get().refreshListing(root);
    void get().refreshGit(root);
  },

  closePalette: () => {
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    searchGen++;
    // 팔레트가 사라지면 그 안의 조회 결과를 볼 곳이 없다 — 진행 중인 git
    // 프로세스(수 분짜리일 수 있다)를 그대로 두지 않고 전부 끊는다.
    const p = get().palette;
    cancelDetailOps(get().detail);
    if (p) {
      cancel(get().gitOpId[p.root]);
      cancelRepoLogOps(get().repoLog[p.root]);
    }
    set({ palette: null, detail: null, search: null, searchLoading: false });
  },

  setQuery: (query) => {
    set((s) => (s.palette ? { palette: { ...s.palette, query, selectedIndex: 0 } } : s));

    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);

    const p = get().palette;
    const backend = useAppStore.getState().appSettings.fileIndexBackend;
    const eligible =
      !!p &&
      backend === "everything" &&
      p.viewMode === "files" &&
      !p.changedOnly &&
      query.trim().length >= SEARCH_MIN_QUERY_LEN;

    if (!eligible) {
      set({ search: null, searchLoading: false });
      return;
    }

    const root = p.root;
    const gen = ++searchGen;
    set({ searchLoading: true });
    searchDebounceTimer = setTimeout(() => {
      void (async () => {
        try {
          // 목록 스캔과 같은 조건으로 검색해야 결과가 어긋나지 않는다.
          const includeIgnored = useAppStore.getState().appSettings.workdirShowIgnored;
          const res = await tauriApi.workdirSearchFiles(root, query, includeIgnored);
          const curP = get().palette;
          // stale 가드: 그 사이 root/query가 바뀌었거나 더 최신 요청이 나갔으면 폐기.
          if (gen !== searchGen || !curP || curP.root !== root || curP.query !== query) return;
          if (!res.usedIndex) {
            set({ search: null, searchLoading: false });
          } else {
            set({
              search: { root, query, files: res.files, truncated: res.truncated },
              searchLoading: false,
            });
          }
        } catch (err) {
          console.warn("workdir: server-side search failed", err);
          if (gen === searchGen) set({ search: null, searchLoading: false });
        }
      })();
    }, SEARCH_DEBOUNCE_MS);
  },

  setSelectedIndex: (index) =>
    set((s) => (s.palette ? { palette: { ...s.palette, selectedIndex: index } } : s)),

  setChangedOnly: (changedOnly) =>
    set((s) => (s.palette ? { palette: { ...s.palette, changedOnly, selectedIndex: 0 } } : s)),

  setViewMode: (mode) => {
    const p = get().palette;
    if (!p) return;
    // 떠나는 뷰가 걸고 있던 조회는 더 보이지 않으므로 취소한다.
    if (p.viewMode !== mode) {
      if (p.viewMode === "log") cancelRepoLogOps(get().repoLog[p.root]);
      else cancelDetailOps(get().detail);
    }
    set({ palette: { ...p, viewMode: mode } });
    // 로그 뷰 최초 진입 시 한 번 로드(캐시에 커밋이 아직 없으면).
    if (mode === "log") {
      const rl = get().repoLog[p.root];
      if (!rl || rl.commits === undefined) void get().loadRepoLog(true);
    }
  },

  refreshListing: async (root, opts) => {
    const force = opts?.force ?? false;
    const includeIgnored = useAppStore.getState().appSettings.workdirShowIgnored;
    const cached = get().listing[root];
    // 무시·숨김 포함 여부가 캐시와 다르면 TTL과 무관하게 다시 스캔한다 —
    // 조건이 다른 목록을 그대로 쓰면 토글을 눌러도 화면이 그대로다.
    const modeChanged = cached !== undefined && cached.includeIgnored !== includeIgnored;
    // TTL 이내면(force가 아닌 한) 스킵 — 캐시가 없으면 isStale이 true를 준다.
    if (!force && !modeChanged && !isStale(cached, LISTING_TTL_MS)) return;
    if (!listingInFlight.begin(root)) return; // 이미 진행 중이면 중복 실행하지 않는다.
    try {
      const res = await tauriApi.workdirListFiles(root, includeIgnored);
      set((s) => ({
        listing: {
          ...s.listing,
          [root]: {
            files: res.files,
            truncated: res.truncated,
            fetchedAt: Date.now(),
            includeIgnored,
          },
        },
      }));
    } catch (err) {
      // 실패 시 기존 캐시·fetchedAt은 그대로 유지한다.
      console.warn("workdir: file listing failed", err);
    } finally {
      listingInFlight.end(root);
    }
  },

  refreshGit: async (root) => {
    // 설정이 꺼져 있으면 조회하지 않고 캐시를 비운다(뱃지 미표시). 진행 중이던
    // 조회가 있으면 함께 끊는다 — 결과를 보여줄 곳이 사라졌기 때문.
    if (!useAppStore.getState().appSettings.gitStatusEnabled) {
      cancel(get().gitOpId[root]);
      set((s) => {
        if (!(root in s.git)) return s;
        const next = { ...s.git };
        delete next[root];
        return { git: next };
      });
      return;
    }
    // in-flight 가드: 타임아웃이 분 단위라 중복 호출이 쌓이면 git 프로세스가
    // 겹겹이 남는다(재오픈/새로고침 연타).
    if (get().gitLoading[root]) return;
    const opId = newOpId();
    set((s) => ({
      gitLoading: { ...s.gitLoading, [root]: true },
      gitOpId: { ...s.gitOpId, [root]: opId },
    }));
    try {
      const res = await tauriApi.workdirGitStatus(root, opId);
      set((s) => ({
        git: { ...s.git, [root]: res },
        gitLoading: { ...s.gitLoading, [root]: false },
        gitOpId: omitKey(s.gitOpId, root),
      }));
    } catch (err) {
      console.warn("workdir: git status failed", err);
      set((s) => ({
        gitLoading: { ...s.gitLoading, [root]: false },
        gitOpId: omitKey(s.gitOpId, root),
      }));
    }
  },

  cancelOp: (opId) => cancel(opId),

  openEntry: (root, relPath, name) => {
    const agentId = get().palette?.agentId ?? "";
    if (isMarkdownPath(relPath)) {
      // 인앱 마크다운 편집기로 위임하고 이 팔레트는 닫는다.
      set({ palette: null, detail: null });
      void useMarkdownStore.getState().openFile(root, relPath, agentId);
      return;
    }
    // 그 외 파일은 외부 에디터(VS Code 등)로 절대경로를 넘겨 연다.
    void tauriApi
      .openInVscode(joinPath(root, relPath))
      .catch((err) => console.warn(`workdir: open file failed: ${name}`, err));
  },
});
