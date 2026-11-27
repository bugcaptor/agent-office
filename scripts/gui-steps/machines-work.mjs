// 머신 연동 업무 분배: 업무 패널의 "호스트에서 받은 업무" 묶음과 다른 머신 캐릭터 묶음, 원격 담당 업무 팝업의
// 저장소 칸(그 머신의 클론 목록·클론이 없을 때의 경고).
//   npm run gui:shot -- --scenario machines-work --script scripts/gui-steps/machines-work.mjs
import assert from "node:assert/strict";

export default async ({ page, shot }) => {
  await page.waitForTimeout(600);
  // 받은 업무는 맨 위 묶음, 다른 머신 캐릭터에게 맡긴 업무는 "이름 · 머신" 묶음에 보인다.
  const groups = await page.locator(".work-agent-group .work-group-name strong").allTextContents();
  assert.ok(groups[0].length > 0 && groups.includes("Ada · studio-pc"), `묶음: ${groups.join(" | ")}`);
  assert.equal(await page.locator(".work-received-chip").count(), 1);
  await shot("machines-work-list");
  await page.locator(".work-card-heading strong", { hasText: "API: add search endpoint" }).click();
  await page.waitForSelector(".work-repo-fields");
  const url = page.locator(".work-repo-fields input").first();
  assert.equal(await url.inputValue(), "github.com/acme/app");
  assert.equal(await page.locator(".work-repo-fields [role=alert]").count(), 0, "그 머신에 있는 저장소면 경고가 없다");
  await shot("machines-work-repo");
  await url.fill("git@github.com:acme/other.git");
  await page.waitForTimeout(100);
  assert.equal(await page.locator(".work-repo-fields [role=alert]").count(), 1, "클론이 없으면 경고");
  await shot("machines-work-repo-missing");
};
