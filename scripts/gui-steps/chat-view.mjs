// 터미널 탭 대화 보기: 전사 복사·저장, 확대, 질문·권한 카드 답, 입력·대기열·중단하고 바로 보내기, 되돌리기.
//   npm run gui:shot -- --scenario working --script scripts/gui-steps/chat-view.mjs
import assert from "node:assert/strict";

export default async ({ page, shot, harness }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  const clipboardText = () => page.evaluate(() => navigator.clipboard.readText());
  const hasClipboardImage = () =>
    page.waitForFunction(async () => (await navigator.clipboard.read()).some((item) => item.types.includes("image/png")));
  await harness.store((s, id) => s.openTerminal(id), "p1");
  await page.waitForTimeout(300);
  await page.locator(".agent-tab-strip-actions button", { hasText: "💬" }).click();
  await page.waitForSelector(".chat-view .chat-ask");
  await page.waitForSelector(".chat-view .chat-image img");
  await shot("chat-view");
  assert.equal(await page.locator(".chat-image-source").first().textContent(), "/tmp/shot.png");
  // 최신이 맨 위다. 도구 결과는 그 호출 블록 안에서 아래에 붙는다.
  assert.match(await page.locator(".chat-block").first().textContent(), /Found the cause/);
  assert.match(await page.locator(".chat-block").last().textContent(), /Look at this screen/);
  assert.match(await page.locator(".chat-block", { hasText: "Read:" }).locator("> *").last().textContent(), /terminal/);

  const user = page.locator(".chat-msg.user").last();
  await user.locator("button.chat-action").click();
  assert.equal(await clipboardText(), "Look at this screen and fix the layout");
  const assistant = page.locator(".chat-msg.assistant").last();
  await assistant.locator(".chat-actions > button").last().click();
  assert.match(await clipboardText(), /\*\*the layout\*\*/);
  await assistant.locator(".markdown-body button.chat-action").click();
  assert.equal((await clipboardText()).trimEnd(), ".panel { display: grid; }");

  const tool = page.locator("details.chat-tool:not(.nested)").last();
  await tool.locator("summary").click();
  await tool.locator("button.chat-action").click();
  assert.equal(await clipboardText(), "src/renderer/terminal/terminal.css");
  const result = page.locator("details.chat-tool.nested").last();
  await result.locator("summary").click();
  await result.locator("button.chat-action").click();
  assert.equal(await clipboardText(), "1\t/* terminal */\n2\t.agent-tab-strip {");
  await page.locator(".chat-ask .chat-text-block button.chat-action").click();
  assert.equal(await clipboardText(), await page.locator(".chat-ask-body").textContent());

  const image = page.locator(".chat-image").first();
  await image.locator(".chat-actions button").first().click();
  await hasClipboardImage();
  await image.locator(".chat-actions button").last().click();
  await page.locator(".chat-msg.assistant button", { hasText: "layout-notes.md" }).click();
  await user.locator("button.chat-action").click();
  await image.locator(".chat-image-open").click();
  const zoom = page.locator(".chat-view-zoom");
  await zoom.locator(".chat-actions button").first().click();
  await hasClipboardImage();
  await zoom.locator(".chat-actions button").last().click();
  assert.deepEqual(await page.evaluate(() => window.__guiHarness.chatDownloads), [
    { agentId: "p1", source: { kind: "image", imageId: "shot" } },
    { agentId: "p1", source: { kind: "file", epoch: 1, seq: 8, index: 0 } },
    { agentId: "p1", source: { kind: "image", imageId: "shot" } },
  ]);
  await shot("chat-view-zoom");
  await page.keyboard.press("Escape");
  assert.equal(await zoom.count(), 0);
  assert.ok(await image.locator(".chat-image-open").evaluate((button) => button === document.activeElement));

  await page.locator(".chat-ask-opt", { hasText: "Grid" }).click();
  await page.locator(".chat-ask .pixel-btn.primary").first().click();
  await page.waitForFunction(() => document.querySelectorAll(".chat-ask").length === 1);
  await page.locator(".chat-ask input").fill("keep dist");
  await page.locator(".chat-ask .pixel-btn.danger").click();
  await page.waitForFunction(() => !document.querySelector(".chat-ask"));
  await page.locator(".chat-view-input textarea").fill("Use grid please");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".chat-msg.user >> text=Use grid please");
  assert.match(await page.locator(".chat-block").first().textContent(), /Use grid please/);
  // 작업 중이면 최신 블록 위에 움직이는 상태 줄이 보이고, "queue:" 메시지는 대기열에 쌓였다 뺄 수 있다.
  await page.waitForSelector(".chat-live.thinking .chat-live-dots");
  await page.locator(".chat-view-input textarea").fill("queue: then run the tests");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".chat-queue-item >> text=queue: then run the tests");
  await shot("chat-view-queue");
  await page.locator(".chat-queue-cancel").click();
  await page.waitForFunction(() => !document.querySelector(".chat-queue"));
  // 작업 중이면 대기열을 건너뛰고 턴을 끊어 바로 보낼 수 있다.
  await page.locator(".chat-view-input textarea").fill("queue: stop and fix the header first");
  await page.locator(".chat-view-input .pixel-btn:not(.primary)").click();
  await page.waitForSelector(".chat-msg.user >> text=queue: stop and fix the header first");
  assert.equal(await page.locator(".chat-queue").count(), 0);
  console.log("answers", JSON.stringify(await page.evaluate(() => window.__guiHarness.chatAnswers)));
  await page.locator(".chat-view-bar .pixel-btn").click();
  await page.waitForFunction(() => !document.querySelector(".chat-view"));
  await shot("chat-view-back");
};
