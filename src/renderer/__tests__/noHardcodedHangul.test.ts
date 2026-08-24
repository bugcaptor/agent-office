// 하드코딩 한글 문자열 금지.
//
// 이 저장소에는 eslint가 없어서, 표시 문자열이 카탈로그를 우회해 코드로
// 돌아오는 회귀는 테스트로 막는다. i18n 이행 중에는 남은 위반을 베이스라인에
// 적어 두고 조여 왔고(98 → 0), **이행이 끝난 지금 베이스라인은 비어 있다.**
// 그래서 실질적으로는 "src/renderer·src/shared의 문자열/JSX 텍스트에 한글 금지"다.
//
// 세 방향으로 지킨다:
//   1) 베이스라인에 없는 파일에서 한글 리터럴이 나오면 실패 (새 위반 금지)
//   2) 베이스라인에 있는데 이제 깨끗해졌으면 실패 (목록에서 빼라 — 래칫 조이기)
//   3) 베이스라인 자체가 비어 있어야 한다 (예외를 늘리려면 이 테스트를 의식적으로
//      깨야 한다 — 조용히 되돌아가지 못하게)
//
// 새 한글 문자열이 필요하면 갈 곳은 셋 중 하나다:
//   - 화면에 나가는 문구      → `src/shared/i18n/locales/ko/*.json` (+ en)
//   - AI에게 보내는 프롬프트  → `src/renderer/i18n/promptProfiles.ts`
//   - 사용자 입력 판정 규칙   → `src/renderer/i18n/textRules.ts`
// 셋 다 스캐너 제외 경로이거나 카탈로그다. 자세한 것은 `docs/i18n-design.md`.
//
// 베이스라인 갱신(이행 중에만 쓰던 것): `UPDATE_HANGUL_BASELINE=1 npx vitest run --dir src`
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { scanHangulLiterals } from "./hangulScan";

const REPO_ROOT = fileURLToPath(new URL("../../..", import.meta.url));
const BASELINE_PATH = fileURLToPath(new URL("./hangulBaseline.json", import.meta.url));

interface Baseline {
  /** 아직 i18n 이행이 끝나지 않은 파일들(저장소 루트 기준 상대 경로). */
  files: string[];
}

describe("하드코딩 한글 문자열 래칫", () => {
  const found = scanHangulLiterals(REPO_ROOT);
  const foundFiles = Object.keys(found).sort();

  if (process.env.UPDATE_HANGUL_BASELINE === "1") {
    it("베이스라인을 갱신했다", () => {
      const next: Baseline = { files: foundFiles };
      writeFileSync(BASELINE_PATH, `${JSON.stringify(next, null, 2)}\n`, "utf8");
      expect(next.files.length).toBeGreaterThanOrEqual(0);
    });
    return;
  }

  const baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8")) as Baseline;
  const allowed = new Set(baseline.files);

  it("i18n 이행이 끝난 파일에 한글 리터럴이 새로 들어오지 않았다", () => {
    const regressions = foundFiles.filter((f) => !allowed.has(f));
    expect(regressions).toEqual([]);
  });

  it("이미 깨끗해진 파일은 베이스라인에서 빠져 있다", () => {
    const stale = baseline.files.filter((f) => !(f in found));
    expect(stale).toEqual([]);
  });

  // 이행이 끝나 베이스라인은 비어 있다. 예외를 다시 만들려면 이 테스트를
  // 의식적으로 깨야 한다 — 파일 하나가 슬그머니 목록에 얹히는 걸 막는 장치다.
  it("베이스라인이 비어 있다 (전면 금지 상태)", () => {
    expect(baseline.files).toEqual([]);
  });
});
