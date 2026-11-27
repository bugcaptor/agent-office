// 머신 연동 호스트: 오피스 위 층 단추(이 머신 · 붙은 머신 · 끊긴 머신), 다른 층(원격 캐릭터), 끊긴 층(흐림),
// 원격 캐릭터 대화 보기(입력·권한 답·세션 단추는 그 머신에 넘긴다).
//   npm run gui:shot -- --scenario machines --script scripts/gui-steps/machines.mjs
import assert from "node:assert/strict";

export default async ({ page, shot, harness }) => {
  await page.waitForTimeout(600);
  await shot("machines-local");
  await page.getByRole("tab", { name: /studio-pc/ }).click();
  await page.waitForTimeout(1200);
  await shot("machines-remote");
  await page.getByRole("tab", { name: /laptop/ }).click();
  await page.waitForTimeout(800);
  await shot("machines-offline");
  await page.getByRole("tab", { name: /studio-pc/ }).click();
  await page.evaluate(async () => {
    const m = await import("/src/renderer/link/remoteFloors.ts");
    m.openRemoteChat("d-1a2b3c4d:agent-a");
  });
  await page.waitForSelector(".remote-chat .chat-ask .pixel-btn.primary");
  await page.waitForTimeout(400);
  await shot("machines-chat");
  // 세션이 도는 원격 캐릭터: 중단·다시 열기·이어하기·닫기. 닫기는 한 번 더 눌러야 간다.
  const session = page.locator(".remote-chat .chat-view-session");
  assert.equal(await session.locator("button").count(), 4);
  const close = session.locator("button.danger");
  await close.click();
  assert.equal(await page.evaluate(() => window.__guiHarness.remoteOps.length), 0, "첫 누름은 확인 상태");
  await close.click();
  await page.waitForTimeout(100);
  assert.deepEqual(await page.evaluate(() => window.__guiHarness.remoteOps), [{ agentId: "d-1a2b3c4d:agent-a", op: "terminate" }]);
  // 권한 요청은 캐릭터 id를 실어 답한다(백엔드가 그 머신에 넘긴다).
  await page.locator(".remote-chat .chat-ask .pixel-btn.primary").click();
  await page.waitForTimeout(100);
  const answers = await page.evaluate(() => window.__guiHarness.chatAnswers);
  assert.equal(answers.at(-1).agentId, "d-1a2b3c4d:agent-a");
  assert.equal(answers.at(-1).action, "allow");
  // 입력칸으로 보낸다.
  await page.locator(".remote-chat .chat-view-input textarea").fill("원격에서 보냄");
  await page.locator(".remote-chat .chat-view-input .pixel-btn.primary").click();
  await page.waitForTimeout(200);
  await shot("machines-chat-sent");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  // 끊긴 머신 캐릭터는 보기만(입력칸·세션 단추 없음).
  await page.getByRole("tab", { name: /laptop/ }).click();
  await page.evaluate(async () => {
    const m = await import("/src/renderer/link/remoteFloors.ts");
    m.openRemoteChat("d-5e6f7a8b:agent-c");
  });
  await page.waitForSelector(".remote-chat .chat-view");
  await page.waitForTimeout(300);
  assert.equal(await page.locator(".remote-chat .chat-view-input").count(), 0);
  assert.equal(await page.locator(".remote-chat .chat-view-session").count(), 0);
  await shot("machines-chat-offline");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  await page.getByRole("tab", { name: /이 머신|This machine/ }).click();
  await page.waitForTimeout(500);
  await shot("machines-back");
};
