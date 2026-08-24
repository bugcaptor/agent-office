// 하드코딩 한글 문자열 래칫(ratchet).
//
// 이 저장소에는 eslint가 없다. i18n 이행이 여러 커밋에 걸쳐 진행되는 동안
// "이미 옮긴 파일이 다시 더러워지는" 회귀만은 막아야 하므로, 남은 위반을
// 베이스라인 파일에 적어 두고 **두 방향으로** 조인다:
//
//   1) 베이스라인에 없는 파일에서 한글 리터럴이 나오면 실패 (새 위반 금지)
//   2) 베이스라인에 있는데 이제 깨끗해졌으면 실패 (목록에서 빼라 — 래칫 조이기)
//
// 마지막 phase에서 베이스라인이 비면 그때부터는 전면 금지가 된다.
//
// 베이스라인 갱신: `UPDATE_HANGUL_BASELINE=1 npx vitest run --dir src`
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
});
