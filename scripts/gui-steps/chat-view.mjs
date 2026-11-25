// 터미널 탭 대화 보기: 전사 복사·저장, 확대, 질문·권한 카드 답, 입력, 되돌리기.
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

  const user = page.locator(".chat-msg.user").first();
  await user.locator("button.chat-action").click();
  assert.equal(await clipboardText(), "Look at this screen and fix the layout");
  const assistant = page.locator(".chat-msg.assistant").first();
  await assistant.locator(".chat-actions > button").last().click();
  assert.match(await clipboardText(), /\*\*the layout\*\*/);
  await assistant.locator(".markdown-body button.chat-action").click();
  assert.equal((await clipboardText()).trimEnd(), ".panel { display: grid; }");

  const tool = page.locator("details.chat-tool").first();
  await tool.locator("summary").click();
  await tool.locator("button.chat-action").click();
  assert.equal(await clipboardText(), "src/renderer/terminal/terminal.css");
  const result = page.locator("details.chat-tool.nested").first();
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
  console.log("answers", JSON.stringify(await page.evaluate(() => window.__guiHarness.chatAnswers)));
  await page.locator(".chat-view-bar .pixel-btn").click();
  await page.waitForFunction(() => !document.querySelector(".chat-view"));
  await shot("chat-view-back");
};
