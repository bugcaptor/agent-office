// 터미널 탭 대화 보기: 전환 버튼, 전사(마크다운·도구·이미지), 질문·권한 카드 답, 입력, 되돌리기.
//   npm run gui:shot -- --scenario working --script scripts/gui-steps/chat-view.mjs
export default async ({ page, shot, harness }) => {
  await harness.store((s, id) => s.openTerminal(id), "p1");
  await page.waitForTimeout(300);
  await page.locator(".agent-tab-strip-actions button", { hasText: "💬" }).click();
  await page.waitForSelector(".chat-view .chat-ask");
  await page.waitForSelector(".chat-view .chat-image img");
  await shot("chat-view");
  await page.locator(".chat-ask-opt", { hasText: "Grid" }).click();
  await page.locator(".chat-ask .pixel-btn.primary").first().click();
  await page.waitForFunction(() => document.querySelectorAll(".chat-ask").length === 1);
  await page.locator(".chat-ask input").fill("keep dist");
  await page.locator(".chat-ask .pixel-btn.danger").click();
  await page.waitForFunction(() => !document.querySelector(".chat-ask"));
  await page.locator(".chat-view-input textarea").fill("Use grid please");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".chat-msg.user >> text=Use grid please");
  await page.locator(".chat-image").first().click();
  await page.waitForSelector(".chat-view-zoom img");
  await shot("chat-view-zoom");
  await page.locator(".chat-view-zoom").click();
  console.log("answers", JSON.stringify(await page.evaluate(() => window.__guiHarness.chatAnswers)));
  await page.locator(".chat-view-bar .pixel-btn").click();
  await page.waitForFunction(() => !document.querySelector(".chat-view"));
  await shot("chat-view-back");
};
