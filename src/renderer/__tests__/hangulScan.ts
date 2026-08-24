// 하드코딩 한글 문자열 스캐너 (테스트 헬퍼 — 그 자체는 테스트가 아니다).
//
// TypeScript 컴파일러 API로 AST를 걸어 **문자열/템플릿 리터럴 안의** 한글만
// 찾는다. 정규식 grep이 아니라 AST를 쓰는 이유는 주석·JSDoc(이 저장소는 한국어
// 주석이 아주 많다)을 원천적으로 배제하기 위해서다. JSX 텍스트 노드(`<b>설정</b>`)도
// 화면에 그대로 나가므로 함께 잡는다.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";

import ts from "typescript";

/** 스캔 대상 루트(저장소 루트 기준 상대 경로). */
export const SCAN_ROOTS = ["src/renderer", "src/shared"];

/**
 * 스캔에서 뺄 경로 조각. 셋 다 "한글 리터럴이 정당한" 곳이다:
 * - `__tests__`: 테스트 픽스처와 기대값
 * - `i18n`: 카탈로그 자체와 언어별 프롬프트 프로필
 * - `*.d.ts`: 타입 선언
 */
const EXCLUDED_SEGMENTS = ["__tests__", `${sep}i18n${sep}`];

const HANGUL = /[가-힣]/;

function isExcluded(path: string): boolean {
  if (path.endsWith(".d.ts")) return true;
  return EXCLUDED_SEGMENTS.some((seg) => path.includes(seg));
}

function walk(dir: string, out: string[]): void {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      walk(full, out);
      continue;
    }
    if (!/\.tsx?$/.test(full)) continue;
    if (isExcluded(full)) continue;
    out.push(full);
  }
}

/** 한 파일 안의 한글 리터럴 개수(0이면 깨끗한 파일). */
function countHangulLiterals(file: string): number {
  const source = ts.createSourceFile(
    file,
    readFileSync(file, "utf8"),
    ts.ScriptTarget.Latest,
    /* setParentNodes */ false,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS
  );

  let count = 0;
  const visit = (node: ts.Node): void => {
    if (
      ts.isStringLiteral(node) ||
      ts.isNoSubstitutionTemplateLiteral(node) ||
      ts.isTemplateHead(node) ||
      ts.isTemplateMiddle(node) ||
      ts.isTemplateTail(node) ||
      ts.isJsxText(node)
    ) {
      if (HANGUL.test(node.text)) count += 1;
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return count;
}

/** 저장소 루트 기준 상대 경로(POSIX 구분자) → 한글 리터럴 개수. 0인 파일은 뺀다. */
export function scanHangulLiterals(repoRoot: string): Record<string, number> {
  const files: string[] = [];
  for (const root of SCAN_ROOTS) walk(join(repoRoot, root), files);

  const out: Record<string, number> = {};
  for (const file of files.sort()) {
    const n = countHangulLiterals(file);
    if (n > 0) out[relative(repoRoot, file).split(sep).join("/")] = n;
  }
  return out;
}
