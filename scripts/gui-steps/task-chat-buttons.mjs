// 상세·루트 팝업의 대화 버튼(docs/task-chat-design.md §대화 열기)과 열린 뒤 탭 줄의 대화 탭.
//   npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/task-chat-buttons.mjs
export default async ({ page, shot }) => {
  const dialog = page.getByRole("dialog");
  const runRow = () => dialog.locator(".work-dialog-run button");
  // 대화가 열리면 터미널 오버레이가 패널을 가리므로 다음 팝업을 열기 전에 닫는다.
  const closeOverlay = async () => { await page.evaluate(async () => (await window.__guiHarness.store()).getState().closeTerminal()); await page.waitForTimeout(200); };
  // 루트 팝업: 실행 줄의 '대화'와 자동 진행 영역의 '현재 태스크에 대해 대화'.
  await page.getByText("Ship search").first().click();
  await page.waitForTimeout(200);
  console.log(`  run row: ${JSON.stringify(await runRow().allTextContents())}`);
  console.log(`  auto-run chat: ${await dialog.locator(".work-auto-run .work-chat-btn").textContent()}`);
  await dialog.locator(".work-dialog-run").scrollIntoViewIfNeeded();
  await shot("root-buttons");
  await dialog.locator(".work-auto-run .work-chat-btn").click();
  await page.waitForTimeout(300);
  console.log(`  dialog after current chat: ${await dialog.count()}`);
  console.log(`  tabs: ${JSON.stringify(await page.locator(".agent-tab").allTextContents())}`);
  await shot("current-chat-tab");
  await closeOverlay();
  // 미저장 변경이 있는 상세 팝업: 저장된 내용 기준 툴팁.
  await page.getByText("Tidy session log filters").first().click();
  await page.waitForTimeout(200);
  await dialog.getByLabel(/본문|Body/).fill("unsaved draft");
  console.log(`  dirty tooltip: ${await dialog.locator(".work-dialog-run .work-chat-btn").getAttribute("title")}`);
  await dialog.locator(".work-dialog-run .work-chat-btn").click();
  await page.waitForTimeout(300);
  const requests = await page.evaluate(() => window.__guiHarness.taskChatRequests.map((r) => `${r.taskId} body=${JSON.stringify(r.context.task.body)}`));
  console.log(`  requests: ${JSON.stringify(requests)}`);
  console.log(`  tabs: ${JSON.stringify(await page.locator(".agent-tab").allTextContents())}`);
  await shot("two-chat-tabs");
  await closeOverlay();
  // 상한: 살아 있는 대화 4개에서 다섯 번째 태스크는 버튼을 막고 문구를 보인다.
  await page.evaluate(async () => { const s = await window.__guiHarness.taskChatStore(); await s.openTaskChat("w1"); await s.openTaskChat("w3"); });
  await closeOverlay();
  await page.getByText("Rebuild search index").first().click();
  await page.waitForTimeout(200);
  console.log(`  limit disabled: ${await dialog.locator(".work-dialog-run .work-chat-btn").isDisabled()}`);
  console.log(`  limit hint: ${await dialog.locator("#work-chat-hint").textContent()}`);
  await dialog.locator("#work-chat-hint").scrollIntoViewIfNeeded();
  await shot("limit-hint");
};
