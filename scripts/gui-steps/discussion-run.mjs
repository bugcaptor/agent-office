// Discussion run controls: a paused discussion's reason and resume/end, a moderator question waiting for the
// person (highlighted card, answer box), then ending it and starting the ready discussion (simultaneous first round).
//   npm run gui:shot -- --scenario discussions --script scripts/gui-steps/discussion-run.mjs
export default async ({ page, shot, harness }) => {
  await page.locator(".discussion-card").first().waitFor();
  const heading = (topic) => page.locator(".discussion-card-heading", { hasText: topic });
  const closeDialog = () => page.evaluate(async () => (await window.__guiHarness.discussionStore()).useDiscussionStore.getState().openDialog(null));
  await heading("meeting room go").click();
  await page.waitForTimeout(200);
  await shot("discussion-run-paused");
  await closeDialog();
  await page.evaluate(async () => {
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-running");
    await useDiscussionStore.getState().commit({ ...d, state: "waiting_user", run: { phase: "user", turns: [], seen: d.run.seen, question: "Is the north corner free on Fridays?" } });
  });
  await page.waitForTimeout(200);
  console.log(`attention cards: ${await page.locator(".discussion-card.attention").count()}`);
  await shot("discussion-run-question-card");
  await heading("meeting room go").click();
  await page.waitForTimeout(200);
  await page.locator(".discussion-run-question textarea").fill("Yes, it is free.");
  await shot("discussion-run-question-detail");
  await page.getByRole("button", { name: /^(끝내기|End)$/ }).click();
  await page.getByRole("button", { name: /^(정말 끝내기|Really end)$/ }).click();
  await page.waitForTimeout(200);
  await closeDialog();
  await heading("Name the meeting room").click();
  await page.waitForTimeout(200);
  await shot("discussion-run-ready");
  // 하네스 설정은 관찰이 꺼져 있다(시작 막힘 문구). 켜면 시작할 수 있다.
  await page.evaluate(async () => {
    const store = await window.__guiHarness.store();
    store.setState((s) => ({ appSettings: { ...s.appSettings, observerEnabled: true } }));
  });
  await page.waitForTimeout(100);
  await page.getByRole("button", { name: /^(시작|Start)$/ }).click();
  await page.waitForTimeout(500);
  console.log(`state: ${await page.evaluate(async () => (await window.__guiHarness.discussionStore()).useDiscussionStore.getState().discussions.find((x) => x.id === "disc-ready").state)}`);
  console.log(`tabs: ${await page.locator(".agent-tab-chat-name").allInnerTexts()}`);
  await shot("discussion-run-started");
};
