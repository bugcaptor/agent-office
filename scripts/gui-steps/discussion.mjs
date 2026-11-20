// Discussions: cards in the work panel, the detail view, and the open-a-discussion dialog filled to 1 moderator + 4 debaters.
//   npm run gui:shot -- --scenario discussions --script scripts/gui-steps/discussion.mjs
export default async ({ page, shot }) => {
  await page.locator(".discussion-card").first().waitFor();
  await shot("discussion-cards");
  await page.locator(".discussion-card-heading", { hasText: "Store discussions" }).click();
  await page.waitForTimeout(200);
  await shot("discussion-detail");
  await page.getByRole("dialog").locator(".work-dialog-close").click();
  await page.locator(".work-discussion-btn").click();
  const dialog = page.getByRole("dialog");
  await dialog.locator("input").first().waitFor();
  await page.waitForTimeout(100);
  const focused = await page.evaluate(() => document.activeElement?.getAttribute("placeholder") ?? document.activeElement?.tagName);
  console.log(`discussion dialog focus: ${focused}`);
  await shot("discussion-new-empty");
  // Esc로 닫히고 다시 열린다.
  await page.keyboard.press("Escape");
  await page.waitForTimeout(100);
  console.log(`dialog after Escape: ${await page.getByRole("dialog").count()}`);
  await page.locator(".work-discussion-btn").click();
  await dialog.locator("input").first().waitFor();
  await dialog.locator("input").first().fill("회의실을 맵 어디에 둘까");
  await dialog.locator("textarea").first().fill("본업이 쉬는 캐릭터만 회의실로 간다. 맵은 지금 사무실 한 칸뿐이다.");
  const add = dialog.getByRole("button", { name: /토론자 추가|Add debater/ });
  for (let i = 0; i < 4; i++) await add.click();
  // 두 번째 토론자에게 악마의 대변인 역할을 준다(역할 기본 CLI로 다시 채워진다).
  const rows = dialog.locator(".discussion-row");
  await rows.nth(2).locator("select").nth(1).selectOption({ label: "악마의 대변인" });
  await page.waitForTimeout(200);
  await shot("discussion-new-filled");
  await dialog.locator(".discussion-limit-options > summary").click();
  await shot("discussion-new-limits");
  await dialog.getByRole("button", { name: /^(만들기|Create)$/ }).click();
  await page.waitForTimeout(200);
  await shot("discussion-created");
};
