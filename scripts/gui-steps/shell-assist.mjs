// 셸 캐릭터: 설정 › 셸 캐릭터 탭과 터미널 위 말 → 명령 입력줄.
//   npm run gui:shot -- --scenario working --script scripts/gui-steps/shell-assist.mjs
export default async ({ page, shot }) => {
  await page.evaluate(async () => {
    const s = (await window.__guiHarness.store()).getState();
    s.updateAgent("p1", { shellAssist: true });
    s.openModal({ kind: "settings" });
  });
  await page.getByRole("tab", { name: /셸 캐릭터|Shell characters/ }).click();
  await page.waitForTimeout(200);
  await page.getByRole("button", { name: /연결 확인|Check connection/ }).click();
  await page.waitForTimeout(200);
  await shot("shell-settings");
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().closeModal());
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().openTerminal("p1"));
  await page.waitForTimeout(300);
  await page.locator(".shell-assist-bar input").fill("큰 파일 10개");
  await page.locator(".shell-assist-bar button").click();
  await page.waitForTimeout(300);
  console.log(`  bar msg: ${await page.locator(".shell-assist-msg").textContent()}`);
  await shot("shell-terminal-bar");
};
