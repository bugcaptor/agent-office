// Decision memo: a concluded discussion waits for the person (highlighted card), its detail shows the memo
// (sanitized Markdown) with "open in browser", and confirming the conclusion makes it finished.
//   npm run gui:shot -- --scenario discussions --script scripts/gui-steps/discussion-memo.mjs
export default async ({ page, shot }) => {
  await page.locator(".discussion-card").first().waitFor();
  const card = page.locator(".discussion-card", { hasText: "weekly digest" });
  console.log(`review card attention: ${await card.evaluate((el) => el.classList.contains("attention"))}`);
  console.log(`review card status: ${await card.locator(".discussion-run-status").innerText()}`);
  await shot("discussion-memo-card");
  await card.locator(".discussion-card-heading").click();
  await page.locator(".discussion-memo-body").waitFor();
  await page.waitForTimeout(200);
  console.log(`memo headings: ${await page.locator(".discussion-memo-body h2").allInnerTexts()}`);
  await shot("discussion-memo-detail");
  await page.locator(".discussion-memo-head button").click();
  await page.waitForTimeout(100);
  console.log(`opened in browser: ${await page.evaluate(() => window.__guiHarness.discussionMemosOpened.join(","))}`);
  await page.getByRole("button", { name: /^(결론 확인|Confirm conclusion)$/ }).click();
  await page.waitForTimeout(200);
  console.log(`state after confirm: ${await page.evaluate(async () => (await window.__guiHarness.discussionStore()).useDiscussionStore.getState().discussions.find((x) => x.id === "disc-review").state)}`);
  await shot("discussion-memo-confirmed");
};
