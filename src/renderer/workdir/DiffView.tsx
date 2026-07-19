// src/renderer/workdir/DiffView.tsx
//
// unified diff 텍스트를 새 npm 의존성 없이 자체 파싱해 색상 렌더한다(마크다운이
// marked를 자체 렌더하는 철학과 동일 — 이슈 #11 후속). 줄 종류만 구분해:
//   diff/index/---/+++/new file 등 → 메타(흐리게), @@ → hunk 헤더(강조),
//   '+' → 추가(초록), '-' → 삭제(빨강), 그 외 → 문맥.
import { useMemo } from "react";

/** 한 줄의 diff 종류에 맞는 CSS 클래스를 고른다. */
function classifyDiffLine(line: string): string {
  // hunk 헤더는 '+'/'-' 판정보다 먼저.
  if (line.startsWith("@@")) return "wd-dl wd-dl-hunk";
  // 파일 헤더/메타(+++/---도 여기; 실제 추가/삭제 줄은 단일 +/-).
  if (
    line.startsWith("diff --git") ||
    line.startsWith("index ") ||
    line.startsWith("--- ") ||
    line.startsWith("+++ ") ||
    line.startsWith("new file") ||
    line.startsWith("deleted file") ||
    line.startsWith("old mode") ||
    line.startsWith("new mode") ||
    line.startsWith("similarity ") ||
    line.startsWith("rename ") ||
    line.startsWith("copy ") ||
    line.startsWith("Binary files")
  ) {
    return "wd-dl wd-dl-meta";
  }
  if (line.startsWith("+")) return "wd-dl wd-dl-add";
  if (line.startsWith("-")) return "wd-dl wd-dl-del";
  return "wd-dl";
}

/** unified diff 텍스트를 줄 단위 색상 블록으로 렌더한다. */
export function DiffView({ diff }: { diff: string }) {
  // 마지막 개행으로 생기는 빈 원소는 버린다.
  const lines = useMemo(() => {
    const arr = diff.split("\n");
    if (arr.length > 0 && arr[arr.length - 1] === "") arr.pop();
    return arr;
  }, [diff]);

  return (
    <div className="wd-diff" role="group" aria-label="변경 내용">
      {lines.map((line, i) => (
        // eslint-disable-next-line react/no-array-index-key -- diff 줄은 위치가 곧 정체성.
        <div key={i} className={classifyDiffLine(line)}>
          {line === "" ? " " : line}
        </div>
      ))}
    </div>
  );
}
