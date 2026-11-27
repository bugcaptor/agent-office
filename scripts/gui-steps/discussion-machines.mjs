// 머신 간 팀 토의(머신 연동 호스트).
//   npm run gui:shot -- --scenario meeting-machines,discussions-machines --script scripts/gui-steps/discussion-machines.mjs
// meeting-machines: 다른 머신 참가자(studio-pc의 Ada·Bo)가 회의실 의자에 분신으로 앉고 회의록 보드에 `이름 · 머신`으로 나온다.
// discussions-machines: 토의 열기 대화상자의 팀원 고르기에 다른 머신 캐릭터가 나오고, 고르면 폴더 안내가 붙는다.
export default async ({ page, shot }) => {
  if (page.url().includes("scenario=meeting-machines")) {
    await page.waitForTimeout(1500);
    await page.evaluate(async () => {
      const { useDiscussionStore } = await window.__guiHarness.discussionStore();
      const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-machines");
      const now = Date.now();
      const said = (min, kind, agentId, text) => ({ at: now - min * 60_000, round: kind === "opening" ? 1 : 2, kind, agentId, text });
      await useDiscussionStore.getState().commit({
        ...d, state: "running", round: 2, startedAt: now - 10 * 60_000, activeSince: now - 10 * 60_000, updatedAt: now,
        transcript: [
          said(9, "opening", "p3", "Build on this machine; signing keys live here."),
          said(9, "opening", "d-1a2b3c4d:agent-a", "studio-pc has the faster disk — **CI cache** is already warm there."),
          said(9, "opening", "d-1a2b3c4d:agent-b", "Either works if the artifact upload is scripted."),
          said(4, "moderator", "p1", "- Ada · studio-pc: Can signing move to studio-pc safely?"),
        ],
        run: { phase: "speakers", seen: {}, turns: [{ agentId: "d-1a2b3c4d:agent-a", question: "q", sentAt: now - 60_000 }] },
      });
    });
    // 쉬던 캐릭터가 회의실까지 걷는 시간.
    await page.waitForTimeout(14000);
    await shot("meeting-machines");
    return;
  }
  await page.locator(".work-discussion-btn").click();
  const dialog = page.getByRole("dialog");
  await dialog.locator("input").first().waitFor();
  await dialog.locator("input").first().fill("Which machine builds the release?");
  await dialog.getByRole("button", { name: /토론자 추가|Add debater/ }).click();
  const rows = dialog.locator(".discussion-row");
  const options = await rows.nth(1).locator("select").first().locator("option").allTextContents();
  console.log(`member options: ${JSON.stringify(options)}`);
  await rows.nth(1).locator("select").first().selectOption({ label: "Ada · studio-pc" });
  await page.waitForTimeout(200);
  await shot("discussion-new-remote");
};
