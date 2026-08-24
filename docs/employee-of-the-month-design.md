# 이 달의 우수사원 (Employee of the Month) 설계

상태: 정본 — 2026-08-24 신설. kbm #2hx.

한 달에 한 번 우수사원을 뽑아 시상 화면에 초상·통계·순위표를 보여주고, 캐릭터가 자기 성격과 지난달 일기를 바탕으로 수상 소감을 말하게 한다. 수상 내역과 소감은 영구 기록되어 나중에 열람할 수 있다.

## 1. 원칙

- **선정은 결정적(deterministic)이다.** LLM에 맡기지 않는다 — 재현 불가·테스트 불가하고 매달 비용이 든다. LLM은 소감 생성에만 쓴다.
- **확정은 write-once다.** 한 달치 레코드가 생기면 재계산·덮어쓰기를 하지 않는다. 소감을 만든 뒤 수상자가 바뀌는 사태를 원천 차단한다. 유일한 후속 변경은 `speeches` append.
- **레코드는 자기완결적이다.** 캐릭터를 삭제하거나 개명하거나 초상을 바꿔도 과거 시상 화면이 온전히 재구성되어야 한다.
- 원천 데이터는 전부 기존 것을 재사용한다. 새로 만드는 것은 수상 기록 영속화·선정 로직·소감 생성기·시상 UI·씬 연출뿐이다.

## 2. 선정 규칙 (rulesVersion 1)

가중합이 아니라 **사전식(lexicographic) 비교**다. 가중치 합성은 단위 정규화가 그 달의 후보 구성에 의존해 설명하기 어렵고, "가장 오래 일한 사원"이라는 직관적 서사가 낫다. `aggregate()`의 기본 정렬(workedMs 내림차순)과도 일치한다.

| 순서 | 기준 | 방향 |
|---|---|---|
| 1 | `workedMs` | 내림차순 |
| 2 | `turns` | 내림차순 |
| 3 | `activeDays` | 내림차순 |
| 4 | `agentId` | 오름차순 (최종 결정성 보장) |

**후보 자격**

| 조건 | 처리 | 근거 |
|---|---|---|
| `deleted === true` | 제외 | 현재 프로필이 없어 초상·성격이 없다. 시상 화면을 구성할 수 없다 |
| 봇 모드 캐릭터 | 제외 | 자동 실행 봇이 작업시간을 매달 독식하면 기능의 재미가 죽는다 |
| `activeDays < 3` | 제외 | 월말에 만든 캐릭터·1회성 실험이 우연히 1위가 되는 왜곡 방지 |
| `workedMs < 30분` | 제외 | 위와 같음 |
| `clockedOut` | **제외하지 않음** | 현재 상태일 뿐 그 달의 실적과 무관하다 |
| 연속 수상 | **허용** | 페널티는 과설계. 실제로 제일 일한 캐릭터가 받는 게 자연스럽다 |

"생성일 기준 제외"는 별도로 두지 않는다 — 최소 활동 임계값이 같은 효과를 내면서 규칙이 하나 줄어든다.

**경계**: 자격자 1명 → 그대로 수상. 자격자 0명(전원 임계 미달 포함) → `winner: null` 레코드를 **기록한다**. 기록해 두어야 매 부팅마다 그 달을 재계산하지 않는다.

월 경계는 `aggregate()`와 동일하게 **로컬 타임존**(`localDayCalendar`) 기준이다. 테스트는 `fixedOffsetCalendar(540)`로 못박는다.

## 3. 확정 시점

- **자동 확정**: 렌더러 부팅 시, 그리고 시상 다이얼로그를 열 때 보정으로 한 번 더 `ensureFinalized()`를 돌린다. 사용자가 버튼을 눌러야 한다면 누르는 걸 잊는 순간 기능이 죽는다.
- **소급**: `session-events/v1/YYYY-MM-DD.jsonl`이 날짜별로 남아 있으므로 지난 달을 언제든 재계산할 수 있다. 앱을 한 달 내내 안 켰어도 나중에 확정된다. 소급 범위는 **최근 12개월로 캡** — 무한 소급은 초기 부팅만 느리게 하고 실익이 없다. 이벤트가 없는 달은 즉시 `winner: null`로 확정되므로 비용은 한 번뿐이다.
- **진행 중인 달**: 저장하지 않는다. 다이얼로그를 열 때 라이브로 `aggregate` + `pickWinner`를 돌려 "이번 달 잠정 선두" 배너만 보여준다.
- 앱을 켜 둔 채 월이 바뀌는 경우 별도 타이머는 두지 않는다. 다음 부팅이나 다이얼로그 오픈에서 잡히면 충분하다.

## 4. 데이터 모델

**저장 위치**

| 경로 | 내용 |
|---|---|
| `<app_data>/awards/awards.json` | 시상 레코드 전체(단일 문서) |
| `<app_data>/awards/portraits/<YYYY-MM>.png` | 확정 시점 수상자 초상 스냅샷 |

JSONL append가 아니라 **단일 JSON + 원자적 쓰기(temp+rename, `png_store.rs` 선례)**를 쓴다. 레코드가 연 12건으로 극소량이고, 소감 append라는 "기존 레코드 수정"이 있어 append-only JSONL은 fold 로직이 필요해 오히려 복잡하다.

**스키마** — 정본은 `src/shared/types/awards.ts`, `src-tauri/src/types.rs`가 serde 미러다.

```ts
interface AwardsFile { version: 1; awards: AwardRecord[] }  // month 오름차순

interface AwardRecord {
  month: string;                // "YYYY-MM"(로컬), 파일 내 유일 키
  decidedAt: number;            // epoch ms
  rulesVersion: number;         // 선정 규칙 버전 — 규칙이 바뀌어도 과거 해석 가능
  winner: AwardWinner | null;   // null = 자격 후보 없음
  leaderboard: AwardStanding[]; // 상위 5, 확정 시점 스냅샷
  speeches: AwardSpeech[];      // 재생성 시 append, 마지막 원소가 대표 소감
}

interface AwardWinner {
  agentId: string;
  name: string; role: string; archetype?: string;  // 확정 시점 스냅샷
  hasPortrait: boolean;                            // 초상 스냅샷 존재 여부
  stats: { workedMs; turns; toolEvents; activeDays; tokensIn; tokensOut; costUsd }
}

interface AwardStanding { agentId; name; workedMs; turns; activeDays }
interface AwardSpeech { at: number; provider: string; text: string }
```

**스냅샷 범위**: 이름·역할·아키타입·통계·순위표(각 행의 이름 포함)·초상 PNG 복사본. 이것으로 캐릭터 삭제·개명·초상 교체와 무관하게 과거 시상 화면을 재구성한다. `personalityPrompt`는 스냅샷하지 않는다 — 소감 생성 시점의 현재 값을 쓰면 되고, 캐릭터가 삭제되면 소감 **생성 버튼만** 비활성화된다(이미 생성된 소감 텍스트는 남는다).

**IPC** (4개). `finalize_award`가 초상 복사까지 내부에서 처리한다 — 렌더러가 파일 전체를 왕복 저장하는 경합을 피한다.

```
load_awards()                                        -> AwardsFile
finalize_award(record, portraitAgentId?)             -> AwardsFile   // upsert-if-absent
append_award_speech(month, speech)                   -> AwardsFile
load_award_portrait(month)                           -> string | null  // base64
```

`month`가 파일명에 쓰이므로 백엔드가 `^\d{4}-(0[1-9]|1[0-2])$`를 검증한다(경로 주입 방지, `diary_store.rs`의 `validate_id` 선례). `version`이 지원 범위보다 크면 로드를 거부하고 파일을 보존한다(덮어쓰지 않는다).

## 5. 수상 소감

`SummaryPurpose::Diary`를 재사용한다. 소감은 일기와 규모가 같다(입력 수천 자, 출력 한 문단, 120초 타임아웃·경량 모델로 충분). 새 variant는 provider별 모델 함수 여러 곳과 설정 오버라이드까지 건드리면서 얻는 차이가 없다.

**프롬프트 조립** (`src/renderer/awards/speechGenerator.ts`, `diaryGenerator.ts:73-124` 선례)

- 시스템: 이 달의 우수사원으로 선정된 캐릭터로서 1인칭 수상 소감을 2~4문장으로. 캐릭터 말투 유지, 통계 수치를 하나쯤 자연스럽게 언급 가능. 메타 발언·마크다운 금지.
- 유저 텍스트: `[성격]` / `[수상 정보]`(월·작업시간·턴·활동일·통산 수상 횟수) / `[지난달 일기]`.
- **일기 발췌**: `loadDiary(agentId)` → `at`이 해당 월인 항목만 필터 → **균등 간격 샘플링**(월 전체 흐름을 반영하는 것이 목적이므로 최신 우선보다 낫다). 0편이면 "(일기 없음)"이고 통계만으로 생성한다.

**분량은 프런트에서 기계적으로 정한다.** 백엔드 요약기는 `Diary` 목적 입력을 `TEXT_MAX_CHARS`(2,000자)에서 head 60% + `…(중략)…` + tail 40%로 자른다(`summarizer/mod.rs`의 `cap_text`). 그 절단에 걸리면 **월 가운데가 통째로 날아가** 균등 샘플링이 무의미해진다. 그래서:

| 축 | 상한 | 근거 |
|---|---|---|
| 프롬프트 총량 | `PROMPT_BUDGET_CHARS` 1,900자 | 백엔드 2,000자보다 작게 — `cap_text`가 안전망으로만 남는다 |
| `[성격]` | 300자 | 성격이 길어도 일기 예산을 다 먹지 못하게 |
| `[지난달 일기]` | 남는 예산 전부(기본 상한 1,500자) | 고정 블록을 확보한 뒤 나머지를 전부 준다 |
| 편수 | 최대 10편 | 편수를 위에서부터 시도해 **예산에 실제로 들어맞는 가장 많은 편수**를 쓴다 |
| 편당 | `floor(예산/편수) - 접두` (최소 60자, 최대 200자) | 편수↑ → 편당↓. 30편을 한 줄씩 늘어놓는 것보다 10편을 문장째로 읽히게 두는 쪽이 낫다 |
| 출력 | `clampSpeech` — 4문장 / 240자 | OpenRouter만 `max_tokens`가 있고 CLI provider는 상한이 없다. 문장 단위로 버려 끝을 깔끔하게 남기고, 한 문장이 통째로 넘칠 때만 글자 절단 |

절단은 전부 **문장 경계 우선**(`cutAtSentence`)이다 — 문장 중간에서 뚝 끊긴 조각은 LLM이 그대로 인용해 어색한 소감을 만든다.

| 상황 | 처리 |
|---|---|
| 재생성 | 허용. `speeches`에 append, 이전 소감 보존(UI에서 접어서 열람) |
| 실패·타임아웃 | 저장하지 않음. 사유 문구 + "다시 시도" 버튼. 사유 매핑은 `diaryGenerator` 것을 따름 |
| summarizer OFF | 버튼 비활성 + "설정에서 요약 기능을 켜면 소감을 들을 수 있습니다" |
| 수상자 프로필 삭제됨 | 생성 버튼 비활성(`profile-missing`). 기존 소감은 표시 |
| 중복 클릭 | 월 단위 인플라이트 가드 |

**TTS**: 수상자는 `voiceId`를 갖고 `src-tauri/src/tts/`에 합성 경로가 이미 있어 "음성으로 듣기" 버튼을 낮은 비용으로 붙일 수 있다. **v1 범위에서는 제외**한다(요구사항의 "듣기"는 텍스트 표시로 충족). 후속 과제.

## 6. UI

전역 패널이므로 **ModalState 패턴**을 쓴다(ProfileDialog·AnalyticsDialog 관례). `store/types.ts`의 `ModalState` 유니온에 `{ kind: "awards" }` 추가 → `openModal` → `App.tsx`의 `.modal-root`에 `AwardsDialog` 상시 마운트. 트리거는 `BottomBar`의 트로피 버튼.

```
┌────────────────────────────────────────────────┐
│  ◀  2026-07 이 달의 우수사원  ▶   [월 목록 ▾]   │  월 네비 + 과거 열람 동선
│  (이번 달 잠정 선두: OO — 진행 중)               │  최신 월일 때만
├──────────────┬─────────────────────────────────┤
│  트로피 프레임 │  통계 배지: 작업시간·턴·활동일·비용  │
│  초상(스냅샷)  │  순위표 top5 (수상자 하이라이트)     │
│  이름 / 역할   │  ── 수상 소감 ──                  │
│  "n회 수상"   │  [소감 듣기] → 텍스트 카드          │
│              │  (재생성 / 이전 소감 접기)           │
└──────────────┴─────────────────────────────────┘
```

- `winner: null`인 달은 "이 달은 수상자가 없습니다" 빈 상태.
- 초상은 `load_award_portrait(month)` 스냅샷 우선 → 현재 초상 폴백 → 기본 실루엣.
- `ProfileDialog`에 통산 수상 횟수 뱃지를 노출한다(스토어에서 count만 읽는 소규모 작업).
- 스타일은 `awards.css` 신설, `tokens.css` 전역 토큰만 참조.

## 7. 오피스 씬 연출

수상자 책상에 트로피, 벽에 수상자 사진을 **절차적으로**(에셋 파일 없이 Pixi `Graphics`) 그린다.

**벽 사진은 액자가 아니라 압정으로 꽂은 작은 폴라로이드다.** 첫 구현(28×28 정사각 액자 + 짙은 테두리 + 매트 + 정면 흉상, 벽 **정중앙** tx8-9에 단 하나)이 눈검증에서 "영정사진 같다 / 수령님 초상화 같다"를 받았다. 도상이 정확히 겹쳤기 때문이다 — 정사각 + 어두운 테두리 + 정면 흉상 + 정중앙 단독 배치 + 축하 맥락 표지 0. 세 축을 동시에 바꿔 깬다.

| 축 | 전 | 후 |
|---|---|---|
| 형태 | 정사각 액자(테두리 2px + 매트 2px) | 폴라로이드 카드 — 사진칸 14×14 + **넓은 아래 턱 5px**(이 비대칭이 정체) + 1px 그림자 + 압정 |
| 크기 | 28×28 | 18×21 |
| 배치 | 정중앙 간격(tx8-9) 중앙정렬 | 오른쪽 간격(tx12-13), 그 안에서도 왼쪽 4px — "누가 붙여둔 사진" |
| 맥락 | 없음 | 아래 턱 중앙에 3px 십자 별 + 압정을 축으로 -0.06rad 기울기 |

기울기는 압정을 pivot으로 **카드만** 돈다(압정은 벽에 박힌 것이라 안 돈다). 픽셀아트라 각도를 크게 주면 nearest 스케일 사진에 계단 아티팩트가 보이므로 격식만 깨질 만큼 아주 조금이다.

수상 표현이 벽 사진뿐도 아니다 — 트로피가 수상자 **책상 위**에 올라간다. 벽 사진은 "누가 받았는지"를 알리는 보조 표시라 작아도 된다.

**책상 트로피**(`entities/TrophyOverlay.ts`)도 눈검증을 두 번 반영했다.

- *"트로피 안 보이는데?"* — 정렬 버그였다. 가구는 자기 **아래 모서리**로 y-sort하는데(`TileRenderer.buildFurniture`: `zIndex = (ty+1)*TILE_SIZE`) 트로피는 `zIndex = root.y`를 써서 상판 중앙값(`ty*16+6`)이 상판 정렬값(`ty*16+16`)보다 작았다 → **자기가 올라앉은 책상 뒤로** 들어가 통째로 가려졌다. 이제 `(deskTy+1)*TILE_SIZE + 1`을 쓴다. 남쪽에 선 캐릭터(`(deskTy+1)*16+8`)는 여전히 앞, 북쪽에 앉은 캐릭터는 뒤다.
- *"좌우로 너무 넓다 / 금색밖에 없어서 구분이 안 감"* — 폭 12px → 8px로 줄이고 목 리본·받침 명패에 포인트 색 `trophyRibbon`(팔레트 축 신규)을 넣었지만, 12×12px 안에 컵·손잡이·기둥·받침을 손으로 다 읽히게 그리는 건 도트 예산이 근본적으로 빡빡했다.
- *"차라리 유니코드에서 트로피 찾아서 확대해서 보여주는 게 어때"* — **채택**. 지금 그림의 1순위는 🏆 글리프를 구운 스프라이트이고, 위의 절차적 드로잉은 폴백으로 남았다.

**이모지 굽기**(`gen/emojiTexture.ts`). 시스템 이모지 폰트로 220px에 그린 뒤 글리프 경계로 크롭해 **nearest로 13px 축소**한다. 처음부터 13px로 `fillText`하면 안티에일리어싱에 뭉개져 픽셀아트가 깨진다(눈검증) — 크게 그린 뒤 줄이는 것이 핵심이다. 폰트 디자이너가 최적화해 둔 실루엣(손잡이·잘록한 목·짙은 받침)이 그대로 살아남아 손그림보다 훨씬 트로피로 읽힌다.

| 결정 | 값 | 근거 |
|---|---|---|
| 크기 | 13px(외곽선 포함 15px) | 16px로 구우면 손잡이가 책상 타일 밖으로 나간다 |
| 색 | **원색 그대로** | 회색조 후 테마 컵색 틴트도 렌더해 봤지만 네 테마 전부 탁해지고 pipboy에선 거의 사라진다. 트로피는 씬에서 유일하게 튀어야 하는 소품이다 |
| 외곽선 | 1px, `darken(trophyBase, 0.45)` | 원색 이모지가 웜톤 책상에 묻힌다. `trophyBase`를 그대로 두르면 명도가 비슷해 테두리가 아니라 얼룩으로 보였다(눈검증) — 채널을 0.45로 눌러야 네 테마 전부에서 윤곽으로 읽힌다. 글리프를 줄이는 대신 **텍스처를 사방 1px 키워** 두른다(11px로 줄이면 눈에 띄게 뭉개진다) |
| 폴백 | 절차적 도트 드로잉 | 이모지 폰트 없음 / 2d 컨텍스트 없음(jsdom·헤드리스)이면 `bakeEmojiTexture`가 null을 낸다. 이 계약이 깨지면 트로피가 통째로 사라지므로 테스트로 못 박았다 |

플랫폼 폰트에 의존한다는 점은 남는 한계다 — macOS는 Apple Color Emoji, Windows는 Segoe UI Emoji로 모양이 다르다. 텍스처는 (문자, 크기, 외곽선색)으로 캐시돼 씬 재구축·테마 왕복에서 다시 굽지 않는다.

- 상태 배선은 `OfficeBus`에 awardee 계약을 추가하고 `sessionBridge.ts`(실구현)·`createMockOfficeBus`(테스트)에 구현한 뒤 씬이 구독해 **부분 갱신**한다. 씬 전체 재생성은 하지 않는다. 선례는 `OfficeScene.buildBossDesk()` + `onVacationModeChanged`(휴가팻말)이며, 이것이 A/C↔B 사이의 유일한 확립된 확장 경로다.
- 책상 좌표는 `assignedDeskIndex` → `deskAssignment.assignDesks` → `DeskSlot.seat` → `tileCenterPx(seat)` 체인으로 얻는다. 책상 위 소품의 기존 선례는 `scenes/officeScene.ts`의 `Tile.DeskTop` 케이스(랩탑) — `g.rect().fill()` 절차적 드로잉이며 텍스처가 아니다.
- 벽 장식은 타일 드로잉(`Tile.Wall`)이 아니라 **표시객체**(`entities/AwardFrameOverlay.ts`)가 그린다. 벽이 한 겹(16px)뿐이라 타일에 구우면 내부 콘텐츠가 8×22px로 눌려 "문짝"이 됐다. 배치(위치+카드 크기)의 단일 출처는 `scenes/officeScene.ts`의 `awardFrameRectPx()`. **색은 테마 팔레트 축**(`theme/themes.ts`의 `TILE_PALETTE_KEYS`의 `frameBorder`=그림자·압정·실루엣 / `frameMat`=카드 종이 / `frameSilhouette`=사진칸 바탕·별)을 쓰고, 안에 들어가는 초상만 별도 텍스처로 분리한다.
- 초상 → Pixi 텍스처 경로는 아직 없다(`portraitCache`는 DOM `<img>` 전용). `sprite/spriteCache.ts`의 `decodeSheet(b64)` → canvas → `Texture.from(canvas)` 패턴을 재사용한다.
- 픽셀 그리드는 `TILE_SIZE = 16`, `antialias:false` / `roundPixels:true` / `resolution:1`로 서브픽셀을 원천 차단한다. 난수가 필요하면 `hashStringToSeed(agentId)` + `mulberry32`만 쓴다(결정성 관례).

## 8. 테스트

**vitest** — `selection.test.ts`: 결정성(같은 입력 → 같은 결과, 입력 unmutated), 4단계 동점 전부, 임계 미달 → `winner: null`, deleted/bot 제외, `clockedOut` 포함됨, `fixedOffsetCalendar`로 월 경계·연말연시. `speechGenerator.test.ts`: 월 필터, 균등 샘플링, 예산 역산(편수·편당)·문장 경계 절단, 프롬프트 총량이 백엔드 상한 아래로 유지됨, 출력 클램프(`clampSpeech`), 일기 없음 경로, 실패 사유 매핑. `AwardFrameOverlay.test.ts`: 폴라로이드 형태(아래 턱 > 위 여백), 별, 압정 비회전 + 카드 회전축, 사진 contain. `awardsStore.test.ts`: 기존 레코드 skip, 누락 월만 확정, 12개월 캡, 진행 중인 달 미확정, 인플라이트 중복 가드.

**cargo** — `awards_store`: 라운드트립, temp+rename 원자성, `month` 키 검증 거부, 미래 `version` 로드 거부 + 파일 보존, upsert-if-absent, speech append, 초상 원본 없을 때 `hasPortrait:false`, 복사 성공 시 `load_portrait`가 Some.

**계약** — Rust struct ↔ TS 미러 직렬화 일치(`src/shared/contract-fixtures/`).

## 9. 열어둔 것

- TTS 음성 소감 — v1 제외, 후속.
- 최소 활동 임계(3일·30분)는 사용 패턴에 따라 조정 여지가 있다. 값은 `SelectionOptions`로 주입 가능하게 열어 두었다.
- 새 수상 확정 시 기존 Notification 경로로 알림을 띄우는 것은 선택 사항으로 남겨둔다.
