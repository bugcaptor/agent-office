// Discussion participant tabs: open the ready discussion's two participants as side tabs, then the card's read-only badges.
//   npm run gui:shot -- --scenario discussions --script scripts/gui-steps/discussion-sessions.mjs
export default async ({ page, shot, harness }) => {
  await page.locator(".discussion-card").first().waitFor();
  await shot("discussion-readonly-cards");
  const open = (participant, focus) => page.evaluate(async ([p, f]) =>
    (await (await window.__guiHarness.discussionSessionStore()).openDiscussionSession("disc-ready", p, { focus: f })) ?? "ok", [participant, focus]);
  console.log(`open moderator: ${await open("p2", false)}`);
  console.log(`open debater: ${await open("p1", true)}`);
  console.log(`other discussion: ${await page.evaluate(async () => (await (await window.__guiHarness.discussionSessionStore()).openDiscussionSession("disc-full", "p3", { focus: false })) ?? "ok")}`);
  await page.locator(".agent-tab-chat", { hasText: "🗣" }).first().waitFor();
  await page.waitForTimeout(300);
  console.log(`tabs: ${await page.locator(".agent-tab-chat-name").allInnerTexts()}`);
  await shot("discussion-tabs");
  await page.locator(".agent-tab-chat-close").last().click();
  await page.waitForTimeout(200);
  await shot("discussion-close-confirm");
  await page.getByRole("button", { name: /^(닫기|Close)$/ }).last().click();
  await page.waitForTimeout(200);
  console.log(`tabs after close: ${await page.locator(".agent-tab-chat-name").allInnerTexts()}`);
  await harness.store((s) => s.closeTerminal());
  await page.waitForTimeout(200);
  await page.locator(".discussion-card-heading").last().click();
  await page.waitForTimeout(200);
  await shot("discussion-readonly-detail");
};
