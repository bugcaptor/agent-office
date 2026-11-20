// Transcript tab: the detail dialog opens on the overview; the transcript tab lists every utterance by round (moderator,
// Q&A and notices styled apart), folds long statements, filters by kind and speaker, and "Export" hands the full
// transcript to the backend as Markdown. In the meeting scenario, clicking a minutes-board row opens the transcript tab
// pointing at that participant's latest utterance.
//   npm run gui:shot -- --scenario discussions --script scripts/gui-steps/discussion-transcript.mjs
//   npm run gui:shot -- --scenario meeting --script scripts/gui-steps/discussion-transcript.mjs
export default async ({ page, shot }) => {
  await page.waitForTimeout(800);
  const meeting = await page.evaluate(async () => {
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    return useDiscussionStore.getState().discussions.some((x) => x.id === "disc-meeting");
  });
  if (meeting) return meetingBoard({ page, shot });

  // 긴 발언 하나를 덧붙여 접기를 보인다.
  await page.evaluate(async () => {
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-full");
    const long = ["### Why one file", "", ...Array.from({ length: 16 }, (_, i) => `- point ${i + 1}: keep **writes** behind the store lock`), "", "```ts", "// one writer", "```"].join("\n");
    await useDiscussionStore.getState().commit({ ...d, transcript: [...d.transcript.slice(0, -1), { ...d.transcript[4], text: long, at: d.transcript[4].at + 1 }, d.transcript.at(-1)] });
  });
  await page.locator(".discussion-card", { hasText: "Store discussions" }).locator(".discussion-card-heading").click();
  await page.locator(".discussion-detail-tabs").waitFor();
  console.log(`tabs: ${await page.locator(".discussion-detail-tab").allInnerTexts()} selected=${await page.locator(".discussion-detail-tab.selected").innerText()}`);
  await page.locator(".discussion-detail-tab", { hasText: /속기록|Transcript/ }).click();
  await page.locator(".discussion-transcript-list").waitFor();
  await page.waitForTimeout(200);
  console.log(`rounds: ${await page.locator(".transcript-round").allInnerTexts()}`);
  console.log(`entries: ${(await page.locator(".transcript-entry").evaluateAll((els) => els.map((el) => el.className.match(/kind-(\w+)/)[1]))).join(",")}`);
  await page.locator(".discussion-transcript-list").evaluate((el) => { el.scrollTop = 0; });
  await shot("transcript-tab");
  await page.locator(".transcript-more").first().click();
  await page.waitForTimeout(100);
  await shot("transcript-expanded");
  await page.getByRole("button", { name: /^(사회자|Moderator)$/ }).click();
  await page.getByRole("button", { name: /^(사람과 문답|Q&A with you)$/ }).click();
  console.log(`filtered entries: ${await page.locator(".transcript-entry").count()}`);
  await shot("transcript-filtered");
  await page.getByRole("button", { name: /^(내보내기|Export)$/ }).click();
  await page.waitForTimeout(100);
  const exported = await page.evaluate(() => window.__guiHarness.discussionTranscriptsOpened.at(-1));
  console.log(`exported ${exported.id}:\n${exported.markdown.split("\n").slice(0, 24).join("\n")}`);
};

async function meetingBoard({ page, shot }) {
  await page.evaluate(async () => {
    const { startDiscussion } = await window.__guiHarness.discussionRun();
    const blocked = startDiscussion("disc-meeting");
    if (blocked) console.log(`start blocked: ${blocked}`);
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-meeting");
    const now = Date.now();
    const said = (min, kind, agentId, text) => ({ at: now - min * 60_000, round: kind === "opening" ? 1 : 2, kind, agentId, text });
    await useDiscussionStore.getState().commit({
      ...d, state: "running", round: 2, updatedAt: now,
      transcript: [
        said(9, "opening", "p2", "Keep them in **CHANGELOG.md** at the repo root; tooling already reads it."),
        said(9, "opening", "p3", "A docs/releases/ folder with one file per version scales better."),
        said(9, "opening", "p4", "Users never open the repo. Put them on the website."),
        said(4, "moderator", "p1", "- Cy: How would a per-version folder stay in sync with tags?\n- Dee: Who updates the website?"),
      ],
      run: { ...d.run, phase: "speakers", turns: [{ agentId: "p3", question: "q", sentAt: now - 2 * 60_000 }, { agentId: "p4", question: "q" }] },
    });
  });
  await page.locator(".meeting-board-row-open").first().waitFor();
  await page.waitForTimeout(400);
  // 사회자 다음 행(첫 토론자)을 누른다.
  await page.locator(".meeting-board-row-open").nth(1).click();
  await page.locator(".discussion-transcript-list").waitFor();
  await page.waitForTimeout(300);
  console.log(`dialog: ${JSON.stringify(await page.evaluate(async () => (await window.__guiHarness.discussionStore()).useDiscussionStore.getState().dialog))}`);
  console.log(`focused: ${await page.locator(".transcript-entry.focused").count()} live: ${await page.locator(".transcript-live").allInnerTexts()}`);
  await shot("transcript-from-board");
}
