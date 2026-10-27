// 태스크 대화 탭. 탭 동작만 보려고 스토어로 직접 연다(버튼은 task-chat-buttons.mjs).
//   npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/task-chat.mjs
export default async ({ page, shot }) => {
  const open = (taskId) => page.evaluate(async (id) => (await (await window.__guiHarness.taskChatStore()).openTaskChat(id)) ?? "ok", taskId);
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().openTerminal("p1"));
  await page.waitForTimeout(300);
  console.log(`  open w5: ${await open("w5")}`);
  console.log(`  open w1: ${await open("w1")}`);
  console.log(`  reopen w5 (reuse): ${await open("w5")}`);
  await page.waitForTimeout(300);
  const requests = await page.evaluate(() => window.__guiHarness.taskChatRequests.map((r) => `${r.taskId} cwd=${r.cwd ?? "-"} cli=${r.agent.cli} plan=${r.context.plan?.id ?? "-"}`));
  console.log(`  requests: ${JSON.stringify(requests)}`);
  console.log(`  tabs: ${JSON.stringify(await page.locator(".agent-tab").allTextContents())}`);
  console.log(`  memo button on chat tab: ${await page.locator(".agent-tab-strip-memo").count()}`);
  await shot("chat-tab");
  await page.locator(".agent-tab-chat").first().hover();
  await page.getByRole("button", { name: /대화 끝내기|End chat/ }).first().click();
  await page.waitForTimeout(200);
  await shot("close-confirm");
  await page.locator(".confirm-close-task-chat-dialog .pixel-btn.primary").click();
  await page.waitForTimeout(200);
  console.log(`  tabs after close: ${JSON.stringify(await page.locator(".agent-tab").allTextContents())}`);
  await shot("after-close");
};
