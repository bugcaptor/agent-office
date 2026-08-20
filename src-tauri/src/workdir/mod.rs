// src-tauri/src/workdir/mod.rs
//
// 에이전트 작업 폴더(이슈 #11)를 앱에서 직접 들여다보기 위한 IPC 커맨드들의
// 구현부. markdown.rs와 같은 골격 -- `#[tauri::command]` 얇은 래퍼가 테스트
// 가능한 순수 함수에 위임하고, 에러는 사용자에게 그대로 보여줄 수 있는 한국어
// 문자열이다.
//
// `list_workdir_files`는 markdown.rs의 목록 스캐너를 확장자 필터만 빼고 그대로
// 재현한다: `ignore` 크레이트(WalkBuilder)로 .gitignore를 존중하고 hidden을
// 스킵하며 심링크는 따라가지 않고, MAX_LIST 상한에 걸리면 truncated=true.
//
// `workdir_git_status`는 시스템 `git`을 `status --porcelain=v2 --branch -z`로 딱
// 한 번 호출해 파일별 상태 뱃지와 브랜치 요약을 뽑는다. libgit2(git2 크레이트)를
// 쓰지 않는 이유: 의존성이 무겁고 거대 저장소에서 오히려 느릴 수 있어, 사용자
// 환경의 git 바이너리를 그대로 쓰는 편이 가볍고 예측 가능하다.
//
// "거대 저장소일 수 있다"는 우려는 세 겹으로 막는다:
//   (1) 프런트/설정의 git 상태 on/off 토글,
//   (2) **사용자 취소** -- 프런트가 조회마다 `opId`를 만들어 넘기고, 오래 걸리면
//       "취소" 버튼이 `workdir_git_cancel(opId)`로 자식 git을 죽인다(1차 탈출구),
//   (3) 타임아웃(status 120초 / diff·log 300초) -- UX 제한이 아니라 취소조차
//       하지 않은 채 매달린 자식을 결국 정리하는 **백스톱**이다.
// 예전에는 (3)만 있었고 값도 3초/10초라, 거대 저장소에서는 사용자가 기다릴
// 의사가 있어도 조회가 무조건 중단됐다.
//
// git 바이너리 부재·비(非) git 저장소·타임아웃·사용자 취소는 모두 에러가 아니라
// 정상 응답의 필드(is_repo=false / timed_out=true / canceled=true)로 표현한다 --
// 작업 폴더 보기 자체는 git과 무관하게 항상 성공해야 하기 때문.
//
// 서브모듈 구성(R-3 분할):
// - `git_runner`: git 서브프로세스 실행·취소 레지스트리·경로/커밋 인자 안전장치
// - `status`: `git status --porcelain=v2` 조회·파싱 + 라벨용 브랜치 단독 조회
// - `diff`: diff/파일 히스토리/커밋로그/difftool
// - `listing`: 작업 폴더 파일 목록 스캔·Everything 서버사이드 검색(이슈 #67)
// - `model`: 결과 타입(struct/enum)
// - `commands`: `#[tauri::command]` 래퍼 11개(조회 계열은 spawn_blocking)
//
// lib.rs의 `tauri::generate_handler![...]`가 `workdir::workdir_*` 경로로 커맨드를
// 등록하므로, 아래 `pub use`들은 분할 이전 경로(`crate::workdir::X`)를 그대로
// 보존하기 위한 재수출이다.

mod commands;
mod diff;
mod git_runner;
mod listing;
mod model;
mod status;

pub use commands::*;
// 아래 재수출들은 crate 내부에서 직접 호출되지 않는다(commands.rs가 이미
// `super::diff::X` 형태로 직접 참조) -- 오직 분할 이전 경로
// (`crate::workdir::git_diff_file` 등)를 보존하기 위한 것이라 unused_imports를
// 끈다.
#[allow(unused_imports)]
pub use diff::{git_commit_files, git_diff_commit, git_diff_file, git_file_history, git_repo_log, launch_difftool};
#[allow(unused_imports)]
pub use listing::{list_workdir_files, search_workdir_files};
#[allow(unused_imports)]
pub use model::*;
#[allow(unused_imports)]
pub use status::{collect_git_branch, collect_git_status, parse_porcelain_v2};
