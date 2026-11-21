// 일반 에이전트 탭. 웹 상태판이 여는 것을 스토어로 직접 흉내 낸다.
//   npm run gui:shot -- --scenario working --script scripts/gui-steps/office-agent.mjs
export default async ({ page, shot }) => {
  const open = (opts) => page.evaluate(async (o) => (await (await window.__guiHarness.officeAgentStore()).openOfficeAgent(o)) ?? "ok", opts);
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().openTerminal("p1"));
  await page.waitForTimeout(300);
  console.log(`  open: ${await open({ prompt: "업무 보드를 정리해 줘", cwd: "/work/app", cli: "claude" })}`);
  console.log(`  open again (blocked): ${await open({ prompt: "x", cli: "claude" })}`);
  await page.waitForTimeout(300);
  console.log(`  active stays: ${await page.evaluate(async () => (await window.__guiHarness.store()).getState().activeTerminalAgentId)}`);
  console.log(`  tabs: ${JSON.stringify(await page.locator(".agent-tab").allTextContents())}`);
  await page.locator(".agent-tab-chat-open").first().click();
  await page.waitForTimeout(300);
  await shot("office-tab");
  await page.locator(".agent-tab-chat").first().hover();
  await page.getByRole("button", { name: /일반 에이전트 끝내기|End general agent/ }).first().click();
  await page.waitForTimeout(200);
  await shot("office-close-confirm");
  await page.locator(".confirm-close-office-agent-dialog .pixel-btn.primary").click();
  await page.waitForTimeout(200);
  console.log(`  tabs after close: ${JSON.stringify(await page.locator(".agent-tab").allTextContents())}`);
};
