// 웹 터미널도 데스크톱 TerminalRegistry와 같은 정책을 쓴다. PTY가 보내는
// ESC[3J는 화면이 아닌 스크롤백만 지우므로, 복구된 과거 출력까지 날아가지
// 않게 스트림에서 제거한다. 시퀀스가 청크 경계에 걸리는 경우도 보존한다.

export const ERASE_SCROLLBACK = "\x1b[3J";

function partialTailLength(text: string): number {
  for (let k = ERASE_SCROLLBACK.length - 1; k > 0; k -= 1) {
    if (text.endsWith(ERASE_SCROLLBACK.slice(0, k))) return k;
  }
  return 0;
}

export interface ScrollbackGuard {
  filter(chunk: string): string;
  reset(): void;
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
    reset(): void {
      carry = "";
    },
  };
}
