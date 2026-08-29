// src/renderer/terminal/scrollbackGuard.ts
//
// PTY 스트림에서 `ESC[3J`(스크롤백 지우기)만 걸러낸다.
//
// pi의 기본 TUI(`--tui-mode regular` = TuiMainScreen)는 대체 화면 버퍼를 쓰지
// 않고, 폭/높이가 바뀔 때마다 `fullRender(true)`로 전체를 다시 그린다. 그 첫
// 바이트가 `ESC[2J ESC[H ESC[3J` — 화면 지우기 + **스크롤백 지우기**다
// (pi-tui/dist/tui-main-screen.js). 그래서 창 크기 조절·터미널 뷰 모드 전환처럼
// 진짜 resize가 한 번 나갈 때마다 그때까지의 터미널 히스토리가 통째로
// 사라진다(pi v0.84.2·0.84.4 PTY 실측. resize가 없으면 0회 — 평상시 차등
// 렌더는 멀쩡하다).
//
// 앱이 resize를 아무리 줄여도 창 크기 조절 자체를 없앨 수는 없으므로,
// 여기서 `ESC[3J`만 스트림에서 떼어 낸다. `ESC[2J`(보이는 화면 지우기)와
// `ESC[H`(홈)는 그대로 통과시키므로 pi는 예전처럼 현재 화면을 다시 그리고,
// 그 위의 스크롤백만 살아남는다.
//
// 대가: agent-office 터미널 안에서는 `clear` 등이 스크롤백을 지우지 못한다
// (pi 세션뿐 아니라 전 세션 공통). 앱이 보여 주는 히스토리를 자식 프로세스가
// 지우는 쪽이 더 손해라 이렇게 정했다.
//
// 청크 경계: `ESC[3J`가 두 청크에 걸쳐 도착할 수 있다. 꼬리가 이 시퀀스의
// 진부분 접두사(`ESC`, `ESC[`, `ESC[3`)면 최대 3바이트를 붙들었다가 다음
// 청크 앞에 이어 붙인다. 붙들린 조각은 그 자체로는 아무것도 렌더하지 않으므로
// 화면상 차이가 없지만, 스냅샷(handoff)처럼 "지금까지 렌더된 것"을 읽어야 할
// 때는 `flush()`로 먼저 게워 낸다.

/** 스크롤백 지우기 시퀀스(CSI 3 J). */
export const ERASE_SCROLLBACK = "\x1b[3J";

/** `text` 꼬리가 `ERASE_SCROLLBACK`의 진부분 접두사면 그 길이(1~3), 아니면 0. */
function partialTailLength(text: string): number {
  for (let k = ERASE_SCROLLBACK.length - 1; k > 0; k -= 1) {
    if (text.endsWith(ERASE_SCROLLBACK.slice(0, k))) return k;
  }
  return 0;
}

export interface ScrollbackGuard {
  /** 청크에서 `ESC[3J`를 제거한, 지금 write해도 되는 부분을 돌려준다. */
  filter(chunk: string): string;
  /** 경계 대기 중인 조각을 게워 낸다(없으면 빈 문자열). */
  flush(): string;
  /** 대기 중인 조각(테스트·진단용). */
  readonly pending: string;
}

export function createScrollbackGuard(): ScrollbackGuard {
  let carry = "";
  return {
    filter(chunk: string): string {
      const merged = carry + chunk;
      carry = "";
      let out = merged.split(ERASE_SCROLLBACK).join("");
      const tail = partialTailLength(out);
      if (tail > 0) {
        carry = out.slice(out.length - tail);
        out = out.slice(0, out.length - tail);
      }
      return out;
    },
    flush(): string {
      const held = carry;
      carry = "";
      return held;
    },
    get pending() {
      return carry;
    },
  };
}
