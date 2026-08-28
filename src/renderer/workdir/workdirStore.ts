// src/renderer/workdir/workdirStore.ts
//
// "작업 폴더 보기"(이슈 #11) 전용 zustand 스토어. markdownStore와 같은 관례로
// appStore에서 분리했다 — 이 상태(파일 목록/git 상태 캐시/팔레트)는 오피스
// 씬·세션과 무관한 독립 서브시스템이라 커플링을 피한다.
//
// 이 파일에는 **조립만** 있다. 액션이 38개까지 불어나면서 한 객체 리터럴이
// 800줄이 됐는데, 그 안에는 서로 거의 닿지 않는 세 축이 섞여 있었다. 축마다
// 파일을 나누고 각자의 배경 설명을 그 파일 머리에 옮겼다:
//
//   slices/paletteSlice.ts — 팔레트·파일 목록·git 상태·서버사이드 검색
//   slices/detailSlice.ts  — 우측 상세 페인(변경점/히스토리, 커밋 펼치기)
//   slices/repoLogSlice.ts — 저장소 전체 커밋 로그 브라우저
//   workdirOps.ts          — 진행 중인 git 조회(opId) 취소·페이지 크기
//   workdirTypes.ts        — 상태 모양과 경로 유틸
//
// 셋은 격리된 상태가 아니라 **같은 `WorkdirState`를 나눠 구현할 뿐**이다 —
// `get()`으로 서로의 액션을 그대로 부른다(뷰를 로그로 바꾸면 로그를 로드하고,
// 팔레트를 닫으면 상세·로그의 진행 중 조회를 함께 끊는다).
import { create } from "zustand";

import { createDetailSlice } from "./slices/detailSlice";
import { createPaletteSlice } from "./slices/paletteSlice";
import { createRepoLogSlice } from "./slices/repoLogSlice";
import type { WorkdirState } from "./workdirTypes";

export type {
  WorkdirDetail,
  WorkdirListing,
  WorkdirPaletteState,
  WorkdirRepoLog,
  WorkdirSearchState,
  WorkdirState,
  WorkdirViewMode,
} from "./workdirTypes";
export { isChangedStatus, isMarkdownPath, joinPath } from "./workdirTypes";

export const useWorkdirStore = create<WorkdirState>()((...a) => ({
  ...createPaletteSlice(...a),
  ...createDetailSlice(...a),
  ...createRepoLogSlice(...a),
}));
