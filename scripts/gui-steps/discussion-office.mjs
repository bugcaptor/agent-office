// Meeting room: start the ready discussion (simultaneous first round), let resting participants walk to the meeting
// room while the working one (Bo) stays at the desk and a translucent clone takes Bo's chair, then shoot it in the three
// themes. The minutes board sits right of the room. A later state with utterances fills the board (meeting-minutes),
// a scene without a meeting room docks it in the corner (meeting-docked), and ending the discussion sends everyone back
// (last shot).
//   npm run gui:shot -- --scenario meeting --script scripts/gui-steps/discussion-office.mjs
export default async ({ page, shot }) => {
  await page.waitForTimeout(1500);
  const blocked = await page.evaluate(async () => {
    const { startDiscussion } = await window.__guiHarness.discussionRun();
    return startDiscussion("disc-meeting") ?? null;
  });
  console.log(`start blocked: ${blocked}`);
  // 쉬던 캐릭터가 탕비실에서 회의실까지 걷는 시간(28px/s, 맵 대각 ≈ 300px).
  await page.waitForTimeout(14000);
  const state = await page.evaluate(async () => {
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-meeting");
    return { state: d.state, phase: d.run?.phase, turns: d.run?.turns.map((t) => [t.agentId, t.sentAt !== undefined, t.text !== undefined]) };
  });
  console.log(`discussion: ${JSON.stringify(state)}`);
  for (const theme of ["daylight", "midnight", "sakura"]) {
    await page.evaluate(async (id) => (await window.__guiHarness.store()).getState().setTheme(id), theme);
    await page.waitForTimeout(400);
    await shot(`meeting-${theme}`);
  }
  // 두 번째 라운드: 첫 라운드 공개 뒤 사회자가 Cy·Dee를 지목했고 Cy가 답하는 중이다.
  await page.evaluate(async () => {
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-meeting");
    const now = Date.now();
    const said = (min, kind, agentId, text) => ({ at: now - min * 60_000, round: kind === "opening" ? 1 : 2, kind, agentId, text });
    await useDiscussionStore.getState().commit({
      ...d, round: 2, updatedAt: now,
      transcript: [
        said(9, "opening", "p2", "Keep them in **CHANGELOG.md** at the repo root; tooling already reads it."),
        said(9, "opening", "p3", "A docs/releases/ folder with one file per version scales better."),
        said(9, "opening", "p4", "Users never open the repo. Put them on the website."),
        said(4, "moderator", "p1", "- Cy: How would a per-version folder stay in sync with tags?\n- Dee: Who updates the website?"),
      ],
      run: { ...d.run, phase: "speakers", turns: [{ agentId: "p3", question: "q", sentAt: now - 2 * 60_000 }, { agentId: "p4", question: "q" }] },
    });
  });
  await page.waitForTimeout(600);
  await shot("meeting-minutes");
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().setScene("valley"));
  await page.waitForTimeout(800);
  await shot("meeting-docked");
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().setScene("office"));
  await page.evaluate(async () => {
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-meeting");
    await useDiscussionStore.getState().commit({ ...d, state: "finished", run: undefined, endedAt: Date.now() });
  });
  await page.waitForTimeout(12000);
  await shot("meeting-ended");
};
