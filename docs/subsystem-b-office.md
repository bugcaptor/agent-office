# 서브시스템 B — 오피스 씬 & 절차적 픽셀 캐릭터

상태: 정본 (2026-10-14) — 구현 전 코드 스케치는 [archive/subsystem-b-office-sketch.md](archive/subsystem-b-office-sketch.md)로 분리했다. 이 문서는 현재 코드의 계약을 파일+심볼로 적는다. 코드 블록 대신 심볼을 가리키므로, 세부 값은 해당 심볼이 정본이다. 이번 정리에서 화면을 새로 확인하지는 않았다.

대상 경로: `src/renderer/office/`. 런타임: Tauri v2 웹뷰 안의 PixiJS v8 캔버스. 웹 표준 API만 쓴다.

## 0. 경계와 파일 지도

B가 바깥과 만나는 면은 셋뿐이다.

| 경계 | 파일 + 심볼 | 계약 |
|---|---|---|
| React 진입 | `OfficeCanvas.tsx` — `OfficeCanvas({ bus, profiles, resyncSignal, theme, scene })` | C는 항상 프로필 **전체 목록**을 넘긴다. 씬에 `addAgent`/`removeAgent`/`setPending` 같은 개별 메서드는 없다 |
| 이벤트 | `bus.ts` — `OfficeBus`, `LabelAnchor`, `OfficeAwardee`, `createMockOfficeBus` | 인터페이스는 B가 정하고, 스토어 백킹 구현은 C의 `ipc/sessionBridge.ts::officeBus`가 주입한다 |
| 프로필 형태 | `types.ts` — `AgentProfile` | `id`·`seed`·`assignedDeskIndex` 등만 읽는 좁은 계약. 인덱스 시그니처로 `@shared/types`의 실제 `AgentProfile`을 구조적으로 받는다 |

| 폴더 | 파일 | 역할 |
|---|---|---|
| 루트 | `OfficeScene.ts`, `useOfficeScene.ts`, `OfficeCanvas.tsx`, `bus.ts`, `types.ts`, `awardee.ts` | Pixi 앱 생명주기·카메라, React 브리지, 이벤트 계약, 우수사원 좌석 판정 |
| `map/` | `mapData.ts`, `deskAssignment.ts`, `TileRenderer.ts` | 의미 타일·좌석 유도, 결정적 좌석 배정, 타일 베이크 |
| `scenes/` | `scenes.ts`, `sceneTypes.ts`, `defineScene.ts`, `sceneColor.ts`, `sceneStorage.ts`, `<풍경>Scene.ts` 10개 | 풍경(scene) 레지스트리와 풍경별 GRID·타일 드로잉 |
| `gen/` | `prng.ts`, `palette.ts`, `parts.ts`, `compositor.ts`, `archetypes.ts`, `sheetGen.ts`, `characterFactory.ts`, `spriteResample.ts`, `spriteOverrides.ts`, `minimiOverrides.ts`, `minimiFactory.ts`, `emojiTexture.ts`, `awardPortraitTexture.ts` | 시드 → 스프라이트 시트 → Pixi 텍스처, 커스텀 시트·미니미 연결 |
| `entities/` | `CharacterEntity.ts`, `behaviorFsm.ts`, `ExclamationOverlay.ts`, `ThinkingOverlay.ts`, `MiniAgentsOverlay.ts`, `TrophyOverlay.ts`, `AwardFrameOverlay.ts` | 캐릭터 1명의 표시·거동, 머리 위/벽 오버레이 |
| `world/` | `OfficeWorld.ts`, `pathing.ts` | 엔티티 집합 diff 동기화·bus 반영·줄서기, 그리드↔픽셀·목적지 선택 |

커스텀 스프라이트 업로드·정규화·편집(`src/renderer/sprite/`)과 초상(`src/renderer/portrait/`)은 이 문서 범위 밖이다. 접점은 `gen/spriteOverrides.ts`·`gen/minimiOverrides.ts`의 오버라이드 레지스트리다.

## 1. 씬 아키텍처

### 1.1 레이어와 카메라

`OfficeScene`이 Pixi `Application`과 고정 레이어 셋을 소유한다. `worldContainer` 아래에 `floorLayer`(베이크된 바닥·벽·벽 액자) → `sortableLayer`(가구 + 캐릭터, `sortableChildren = true`, `zIndex = worldY`) → `overlayLayer` 순서다. 가구와 캐릭터를 한 정렬 컨테이너에 두어 책상 앞/뒤 판정을 y-정렬로 해결한다.

카메라는 **고정 + 정수 스케일**이다. `OfficeScene.ts::computeIntegerScale`이 뷰포트에 맵 전체가 들어가는 최대 정수 배율(최소 1)을 고르고, `applyCamera`가 정수 위치로 중앙 정렬한다. 앱은 `antialias: false`, `roundPixels: true`, `resolution: 1`로 만든다. 선명도는 DPR 배율이 아니라 정수 스케일에서 나온다. 팬·줌은 없다.

`applyCamera`는 Pixi 렌더러 크기 대신 캔버스 부모의 `clientWidth/clientHeight`를 직접 잰다. 0×0(최소화·숨은 탭)이면 카메라를 그대로 두고, `visibilitychange`·`focus`로 깨어날 때 다시 잰다. 정해진 스케일은 `OfficeWorld.setRenderScale`과 우수사원 초상 프리필터에도 전달된다(§3.4).

### 1.2 생명주기

| 심볼 | 계약 |
|---|---|
| `OfficeScene.init` | 비동기. 맵 레이어, 책상 히트영역(`buildDeskHitAreas`), 보스 책상(`buildBossDesk`), 공용 서버 랙(`buildRepositoryServer`), 우수사원 표시(`buildAwardDisplays`)를 짓고 ticker를 건다. ticker 콜백은 `world.update(deltaMS)`와 `publishLabelAnchors`만 부른다. 실시간 타이머는 쓰지 않는다 |
| `OfficeScene.destroy` | 멱등. `world.destroy()`(bus 구독 해제)는 `started` 가드보다 **먼저** 무조건 실행한다. Pixi 앱 파기는 `init` 완료 후에만 한다. React StrictMode가 `init` 전에 정리를 부르는 경우를 막는다 |
| `OfficeScene.syncAgents` | `started` 전 호출은 버린다. 초기 동기화는 `useOfficeScene`이 `init().then`에서 `profilesRef`(최신 렌더 값)로 한다. 마운트 시점 클로저를 쓰면 부팅 hydrate가 init 도중에 와서 캐릭터가 하나도 안 그려진 채 굳는다 |
| `OfficeScene.setTheme` | 색만 바꾼다: 배경 + 바닥 재베이크 + 가구 교체 + 팻말 재도색(`repaint`) |
| `OfficeScene.setScene` | 지오메트리까지 바꾼다: `world.setMap` → 히트영역 파기 → `repaint` → 히트영역·보스 책상·서버 랙·우수사원 표시 재구축 → 카메라·라벨 앵커 갱신. `init` 전이면 값만 바꿔 둔다 |

`useOfficeScene.ts::useOfficeScene`은 마운트마다 `<canvas>`를 **새로 만들어** 컨테이너에 붙인다. StrictMode 이중 마운트에서 두 `Application`이 한 캔버스의 WebGL 컨텍스트를 공유하면, 버려진 쪽의 지연 파기가 살아 있는 씬의 컨텍스트를 끊는다(검은 화면 원인). 테마·풍경·프로필은 ref로 최신값을 넘기고, `resyncSignal`(C가 `spritePreviews`+`minimiPreviews`를 묶어 넘긴다)이 바뀌면 다시 동기화한다. 프로필 필드가 안 바뀌는 커스텀 시트 디코드 완료를 잡기 위해서다.

### 1.3 이벤트 버스(`bus.ts::OfficeBus`)

| 방향 | 메서드 | 소비 |
|---|---|---|
| A/C → B | `onNotificationChanged`, `onSessionStateChanged`, `onSubagentCountChanged`, `onVacationModeChanged`, `onAwardeeChanged` | `OfficeWorld`·`OfficeScene`. 구독 즉시 현재 상태를 1회 재생한다(기본값 생략) — 재마운트된 씬도 상태를 받는다 |
| B → C | `emitAgentClicked`, `emitAgentHoverChanged`, `emitDeskClicked`, `emitBossDeskClicked`, `emitRepositoryServerClicked`, `emitLabelAnchorsChanged` | 터미널 열기, 호버 카드, 책상 주인 지정 메뉴, 보스 자리, 저장소 점검, 머리 위 라벨 |
| C ↔ C | `onAgentHoverChanged`, `onDeskClicked`, `onLabelAnchorsChanged` | C 컴포넌트가 B의 방출을 구독하는 짝 |

`OfficeScene` 생성자는 hover·책상 클릭 좌표에 캔버스 `getBoundingClientRect` 원점을 더해 창 좌표로 바꾼다(fixed DOM 메뉴용). 라벨 앵커는 캔버스 좌표 그대로다(`worldToScreen`). `bindHoverGate`는 포인터가 캔버스 위에 있을 때만 Pixi `features.move`를 켜서, 캔버스를 덮은 오버레이 위에서 뒤의 캐릭터가 hover되지 않게 한다.

## 2. 맵과 풍경

### 2.1 의미 타일과 좌석 유도(`map/mapData.ts`)

- `TILE_SIZE = 16`, 맵 20×14. `Tile`은 `Floor/Wall/DeskTop/Rug/Plant/Counter/Table/BossDesk` 8종이다. GRID는 문자열 행을 `L()`로 읽는다(`F/W/D/R/P/C/T/B`).
- `buildSceneMap(grid, breakRoom)`이 `OfficeMap`을 만든다. 좌석(`desks`)·보스 책상 사각형(`bossDesk`)·줄 슬롯(`queueSlots`)은 전부 GRID에서 유도된다. 풍경이 정하는 값은 GRID와 휴게 사각형뿐이다.
- 좌석은 `DeskTop` 쌍의 왼쪽 칸마다 하나이고, 책상 **위쪽** 칸에 앉아 정면(`facing: 'down'`)을 본다. 캐릭터는 책상 뒤에 앉고, `CharacterEntity.ts::SEAT_SINK_PX`만큼 발을 내려 남쪽 책상이 다리를 가린다.
- 줄 슬롯(최대 8)은 보스 책상 하단 행을 따라 늘어선다. 보스 자리가 맵 오른쪽 절반이면 서쪽, 왼쪽 절반이면 동쪽으로 뻗는다.
- `breakRoom`/`bossDesk`/`queueSlots`는 옵셔널이다. 없으면 오피스 기본 상수 `BREAK_ROOM_RECT`/`BOSS_DESK_RECT`/`QUEUE_SLOTS`로 폴백한다(필드 도입 전 테스트 픽스처 호환).
- 걸을 수 있는 칸은 `Floor`와 `Rug`뿐이다(`world/pathing.ts::isWalkable`).

### 2.2 풍경(scene) 레지스트리(`scenes/`)

풍경은 테마(색 축)와 직교한 **무대 축**이다. `scenes.ts::SCENES`/`SCENE_ORDER`에 `office`·`beach`·`valley`·`spaceship`·`castle`·`steppe`·`cruise`·`factory`·`volcano`·`zombie` 10개가 있고 기본은 `DEFAULT_SCENE_ID = "office"`다.

- 계약(`sceneTypes.ts`): `SceneDef = { map, resolve(theme) → SceneRender{ background, drawTile } }`. 모든 풍경이 같은 `Tile` enum과 같은 20×14를 쓰므로 좌석 유도·배정·FSM·줄서기는 풍경을 모른다. 16px 한 칸을 무엇으로 그릴지(`TileDrawFn`)만 풍경 몫이다.
- `map/TileRenderer.ts::TileRenderer`가 공용 기계다. `build()`는 가구 아닌 칸을 한 장으로 굽는다(`cacheAsTexture({ scaleMode: "nearest" })` — 기본 linear는 픽셀아트를 흐린다). `buildFurniture()`는 `FURNITURE_TILES`(책상·화분·카운터·테이블·보스 책상)를 개별 Graphics로 돌려주고 `zIndex`를 미리 단다.
- 색: `office`는 `theme.pixi`를 그대로 쓴다(`officeScene.ts::officeTileDraw`). 나머지는 `defineScene.ts::defineScene`이 한 절차로 조립한다 — 원색 팔레트 → `quietPalette` → `desaturatePalette` → `adaptPalette`(`sceneColor.ts`). 씬 안 텍스트(휴가 팻말)와 보스 명패 색은 테마 팔레트를 쓴다.
- 배경 저작 규칙: 재질은 16px마다 같은 무늬를 반복하지 않고 월드 좌표(`tx·s`, `ty·s`)의 연속 위상을 쓴다. 파도·능선·창·기둥처럼 여러 칸에 걸친 형태로 읽히게 하되, 실제 드로우 사각형은 각 타일 안에 둔다. 바닥은 낮은 대비의 큰 면, 명암은 벽 접점·물가·라운지 외곽에 모은다. 재베이크해도 무늬가 같도록 좌표 기반의 결정적 계산만 쓴다.
- 미리보기: `scripts/render-scenes.mjs`가 10풍경×4테마를 그린다. 실제 Pixi 캐시·오버레이 확인을 대신하지는 않는다.
- 영속: `sceneStorage.ts`(`localStorage "agent-office.scene"`, 잘못된 값은 `office`). UI 쪽 픽커·스토어 배선은 [subsystem-c-ui.md](subsystem-c-ui.md) §6.5.

### 2.3 좌석 배정(`map/deskAssignment.ts::assignDesks`)

`assignDesks(map, agentIds, manualByAgent?)` — 순수·결정적·입력 순서 무관(id 정렬).

1. 수동 지정(`AgentProfile.assignedDeskIndex`)을 먼저 배정한다. 범위 밖 인덱스는 무시하고, 중복 지정은 id 정렬 순 첫 번째만 인정한다. **지정된 책상은 승자 유무와 무관하게 자동 풀에서 빠진다.**
2. 나머지는 `hashStringToSeed(id) % n`에서 시작해 선형 탐사로 빈 책상을 잡는다.
3. 책상이 모자라면 초과 에이전트는 미배정이고 엔티티를 만들지 않는다.

`awardee.ts::resolveAwardeeSeat`도 같은 함수로 수상자 좌석을 구한다. B에 좌석 규칙은 이것 하나다.

## 3. 절차적 캐릭터 생성

파이프라인: `seed → hashStringToSeed(xfnv1a) → mulberry32 → 아키타입 팔레트·파츠 선택 → 사용자 색 오버라이드 → 16×16 4프레임 시트(idle0, idle1, walk0, walk1) → Pixi 텍스처(nearest)`.

### 3.1 순수 계층(`gen/`, Pixi/DOM 비의존)

| 파일 + 심볼 | 계약 |
|---|---|
| `prng.ts` — `hashStringToSeed`, `mulberry32`, `makeRng` | 결정적 PRNG와 `Rng` 헬퍼(`int/range/pick/bool`) |
| `palette.ts` — `generatePalette`, `ramp`, `contrastRatio`, `clampShirtRamp`, `SHIRT_SKIN_MIN_CONTRAST` | HSL 램프(shadow/base/light). 셔츠-피부 대비 ≥ 1.6: 재시도 8회 후 실패하면 `clampShirtRamp`가 hue를 유지한 채 명도를 피부 반대 극단으로 스캔한다(hue만 바꾸는 클램프는 WCAG 가중치 때문에 보장 못 함) |
| `palette.ts` — `applyColorOverrides`, `rampFromColor`, `parseHexColor`, `hasColorOverrides` | 사용자 색(`ColorOverrides`: skin/hair/shirt)을 시드 팔레트 **위에** 얹는다. rng 소비 뒤에 적용하므로 시드 결정성은 그대로다 |
| `parts.ts` — `BODY_BASE_FRONT`, `LEGS_WALK_A/B`, `HAIR_VARIANTS`, `CLOTHES_VARIANTS`, `ACCESSORY_VARIANTS`, `PixelRows` | 16행 문자열 픽셀 데이터. 문자 하나 = 팔레트 슬롯 하나(`compositor.ts::resolveChar`). 헤어 4 × 의상 3 × 액세서리 4 |
| `compositor.ts` — `CELL`(16), `FRAME_ORDER`, `CanvasFactory`, `defaultCanvasFactory`, `composeSpriteSheet`, `composeFramesSheet` | 레이어 합성(휴머노이드) 또는 프레임 직접 지정(비휴머노이드)으로 64×16 시트를 만든다. idle1·walk1은 1px bob, walk는 다리 스왑 |
| `archetypes.ts` — `ARCHETYPES`, `ARCHETYPE_IDS`, `getArchetype`, `pickArchetype`, `resolveArchetype`, `composeArchetypeSheet` | 종족 8종: human·elf·orc·beastfolk·android(layers 경로)와 robot·slime·ghost(frames 경로). **human은 원래 파이프라인과 시드 단위로 바이트가 같다**(회귀 계약). `"auto"`는 별도 해시 스트림(`seed + ":archetype"`)으로 추첨해 팔레트·파츠 rng를 오염시키지 않는다. `ARCHETYPE_IDS` 순서는 추첨 결정성의 기준이라 바꾸지 않는다 |
| `archetypes.ts` — `keyColorsFor`, `basePaletteFor`, `customArchetypeSubject` | 그림 프롬프트에 실리는 키 컬러와 컬러 피커 기본값. 목록 밖 자유 입력 종족은 스프라이트가 human으로 폴백하고 초상·픽셀아트 프롬프트(`portrait/promptBuilder.ts`)의 주제 서술에만 쓰인다 |
| `sheetGen.ts` — `selectLayers`, `generateSheet` | 시드(+아키타입+색) → 시트의 순수 진입점. 마스코트 창이 Pixi 없이 쓰도록 `characterFactory`에서 떼어 냈다(`characterFactory`가 재수출) |

### 3.2 Pixi 계층(`gen/characterFactory.ts`)

- `createCharacterAssets(profile, renderScale?)` → `CharacterAssets{ base, frames, idle, walk, cellSize, descriptor, dispose? }`. 커스텀 시트 오버라이드(`getSpriteOverride`)가 있으면 `assetsFromCustomSheet`, 없으면 `generateSheet` 결과를 nearest 텍스처로 자른다.
- `generateSpritePreview(seed, scale=6, …, archetype, colors)` — C의 프로필 다이얼로그 미리보기용 동기 함수. idle0만 확대한 PNG dataURL을 돌려준다. 캔버스 팩토리 두 개는 테스트 주입 시임이다.
- `downscaledFrameTexture` — 커스텀 고해상 시트를 프레임별로 area 프리필터해 축소한다. 미니미도 재사용한다.

### 3.3 커스텀 시트와 해상도(이슈 #47)

- `gen/spriteOverrides.ts`와 `gen/minimiOverrides.ts`는 agentId → `CanvasImageSource` 레지스트리다. 부팅 로드·삭제는 `src/renderer/sprite/spriteCache.ts`·`minimiCache.ts`가 채운다.
- `gen/spriteResample.ts` — `detailCellSize(n, S) = min(N, 16·S)`, `minimiDetailCellSize(n, S) = min(N, 8·S)`, `areaDownscalePremul`. 카메라 정수 스케일 S가 바뀌면 `OfficeWorld.setRenderScale`이 목표 해상도가 달라진 커스텀 엔티티만 `replaceAssets`/`replaceMinimi`로 텍스처를 갈아 끼운다(FSM·이동 상태는 유지). 절차 생성 스프라이트는 항상 16px이라 S와 무관하다.
- 겉보기 크기는 커스텀 여부와 무관하게 캐릭터 16px(`CharacterEntity.ts::APPARENT_CELL`), 미니미 8px이다.

### 3.4 우수사원 텍스처

`gen/emojiTexture.ts::bakeEmojiTexture`는 🏆 글리프를 크게 그린 뒤 nearest로 줄여 굽는다(플랫폼 폰트 의존, 실패 시 호출부가 절차적 폴백을 든다). `gen/awardPortraitTexture.ts`는 그 달 초상 스냅샷을 벽 폴라로이드 슬롯 크기로 프리필터한다. 연출 계약은 [employee-of-the-month-design.md](employee-of-the-month-design.md) §7.

## 4. 캐릭터 거동

### 4.1 행동 FSM(`entities/behaviorFsm.ts::stepBehavior`)

순수 전이 함수다. 시계·난수를 소유하지 않고 `dtMs`·상태 경과 시간·이번 틱 난수를 주입받는다.

- 상태: `sitting`, `walking`, `breakIdle`, `queueing`.
- 입력(`FsmContext`): `hasPending`, `sessionActive`(starting/running), `shouldQueue`(월드가 계산), `timerMs`, `rand`.
- 출력 요청(`FsmResult`): `requestBreakTarget`, `requestBreakWander`, `requestReturnToDesk`, `requestQueueSlot`.

| 현재 | 조건 | 다음 |
|---|---|---|
| `sitting` | `shouldQueue` | `walking`(줄 슬롯) |
| `sitting` | 세션 활성 또는 알림 대기 | `sitting` 유지 |
| `sitting` | 비활성 상태로 `SIT_LINGER_MS`(2초) 경과 | `walking`(탕비실) |
| `breakIdle` | `shouldQueue` | `walking`(줄 슬롯) |
| `breakIdle` | 세션 활성 또는 알림 대기 | `walking`(자리 복귀) |
| `breakIdle` | 초당 6% 확률 | `walking`(탕비실 안 산책) |
| `queueing` | `!shouldQueue` | `walking`(자리 복귀) |
| `walking` | — | 도착 판정은 `CharacterEntity`가 한다 |

### 4.2 이동과 목적지(`world/pathing.ts`)

`tileCenterPx`, `tileKey`, `isWalkable`, `pickWanderTarget`, `pickBreakTarget(map, rand, occupied)`. `pickBreakTarget`은 `map.breakRoom` 안에서 고르고, `occupied`(다른 캐릭터가 예약한 타일)를 피한다. 전부 점유면 `null`을 돌려주고 호출자가 다음 틱에 재시도한다. 경로 탐색은 없다 — 목표 픽셀로 직선 보간한다(`CharacterEntity` `WALK_SPEED` 28px/s, 도착 임계 0.5px).

### 4.3 CharacterEntity(`entities/CharacterEntity.ts`)

캐릭터 1명의 표시객체·FSM·이동·애니메이션을 소유한다. `AnimatedSprite` 대신 수동 프레임 교체(idle 480ms, walk 140ms)를 쓴다.

- 입력 메서드: `setPending`, `setSessionActive`, `setSubagentCount`, `setSeat`(같은 타일이면 no-op, 다르면 새 좌석으로 걸어감), `setQueueSlot`, `replaceAssets`, `replaceMinimi`.
- 콜백: `onClicked`, `onHover`. `OfficeWorld`가 각각 `bus.emitAgentClicked`/`emitAgentHoverChanged`에 연결한다.
- 탕비실 타일 예약은 `OfficeWorld`가 소유한 공유 `Set`(`breakReservations`)에 엔티티가 넣고 뺀다(`destroy` 포함).
- 오버레이(모두 코드 드로잉, dt 구동, 엔티티가 소유·파괴):
  - `ExclamationOverlay` — 알림 대기 "!" 바운스.
  - `ThinkingOverlay` — 착석·세션 활성·알림 없음일 때 "..." 말풍선. 점멸 주기(숨김 4~7초 / 표시 2~2.5초)는 엔티티가 seeded rand로 정한다.
  - `MiniAgentsOverlay` — 서브에이전트 미니미(§4.6).

### 4.4 월드 관리자(`world/OfficeWorld.ts`)

| 심볼 | 계약 |
|---|---|
| 생성자 | bus의 알림·세션 상태·서브에이전트 수·휴가 모드를 구독해 엔티티와 캐시(`sessionActive`, `subagentCounts`, `pendingIds`)에 반영한다 |
| `syncAgents(profiles)` | diff 동기화: 사라진 에이전트 파괴 → `appearanceKey`가 바뀐 엔티티 재생성 → 좌석을 잃은 엔티티 파괴·좌석이 바뀐 엔티티 `setSeat` → 새 엔티티 생성(캐시 상태 재주입) → `recomputeQueue` |
| `appearanceKey(p)` | archetype·seed·색 오버라이드·`spriteUpdatedAt`·커스텀 시트 유무·`minimiUpdatedAt`·미니미 유무. 빈 색 객체는 "오버라이드 없음"과 같은 키다 |
| `recomputeQueue` | 줄 = `pendingIds`(도착 순) ∩ 엔티티, 휴가 중이면 빈 줄. 슬롯 수를 넘는 사람은 줄에 서지 않는다. 엔티티 없는 pending은 유령 자리를 만들지 않는다 |
| `setMap(map)` | 풍경 교체. 엔티티는 맵 참조를 들고 있어 부분 갱신이 안 되므로 전부 파기 후 재생성하고 탕비실 예약을 비운다. 캐시는 유지돼 상태가 되살아난다 |
| `setRenderScale(S)` | §3.3 |
| `collectLabelAnchors(out)` | 머리 위 앵커(`LABEL_ANCHOR_OFFSET_Y`)를 재사용 Map에 채운다(프레임당 할당 상수) |

결정성: 좌석은 `assignDesks`, 이동 난수는 `mulberry32(hashStringToSeed(id) ^ MOVEMENT_RNG_SALT)`다. 이 파일에 `Math.random`은 없다.

### 4.5 씬 소품

`OfficeScene`이 맵 위 소품을 직접 짓는다. 소품은 히트영역과 함께 `setScene` 때 다시 만든다.

- 책상 히트영역(`buildDeskHitAreas`) → `emitDeskClicked`. 책상 주인 지정 메뉴(`desk/DeskAssignMenu`)가 받는다.
- 보스 책상(`buildBossDesk`) — 히트영역과 휴가 텐트 카드. 팻말 문구는 React 밖이라 `languageChanged`를 구독해 `.text`만 바꾸고, 글씨 배율은 `1/scale`로 상쇄한다.
- 공용 서버 랙(`buildRepositoryServer`) — 모든 풍경의 왼쪽 아래. 클릭하면 `emitRepositoryServerClicked` → 저장소 점검 오버레이([repository-audit-design.md](repository-audit-design.md)).
- 우수사원(`buildAwardDisplays`, `TrophyOverlay`, `AwardFrameOverlay`) — 수상자 좌석 트로피와 벽 폴라로이드. 재구축 직후에는 bus가 다시 발화하지 않으므로 최신 `awardee`·`vacationOn` 값을 씬이 보관해 복원한다.

### 4.6 서브에이전트 미니미(`entities/MiniAgentsOverlay.ts` + `gen/minimiFactory.ts`)

머리 옆에 뜨는 최대 3마리의 작은 분신(`setCount(n)` → `min(n, 3)`). 기본은 **부모 스프라이트 idle0 텍스처를 공유**해 겉보기 8px(부모 16px × `MINI_SCALE_FACTOR` 0.5)로 그린다. 캐릭터별 **미니미 전용 픽셀아트**를 지정할 수 있다. 지정이 없으면 부모 축소판 그대로다.

데이터·저장 경로는 커스텀 스프라이트와 같은 모양이다.

| 축 | 커스텀 스프라이트 | 커스텀 미니미 |
| --- | --- | --- |
| 프로필 필드 | `spriteUpdatedAt` | `minimiUpdatedAt` |
| PNG 저장소 | `<data_dir>/sprites/<id>.png` | `<data_dir>/minimis/<id>.png` |
| IPC | `save/load/delete_sprite` | `save/load/delete_minimi` |
| 이미지 규격 | 4N×N 4프레임 시트 | **단일 N×N 프레임** (N ∈ [16,256]) |
| 오버라이드 레지스트리 | `gen/spriteOverrides.ts` | `gen/minimiOverrides.ts` |
| 부팅 로드/삭제 브리지 | `sprite/spriteCache.ts` | `sprite/minimiCache.ts` |
| 프리뷰 캐시(zustand) | `spritePreviews` | `minimiPreviews` |
| 번들(`.aoc.json`) 필드 | `spritePngBase64` | `minimiPngBase64`(선택, v1 하위호환) |

렌더 규약:

- 겉보기 크기는 **항상 `MINIMI_CELL`(8px)** 이다. `spriteResample.MINIMI_CELL === CELL × MINI_SCALE_FACTOR`를 테스트가 고정한다.
- S-적응 프리필터는 캐릭터와 **겉보기 크기만 다르게** 적용한다: `D = min(N, 8·S)`(`minimiDetailCellSize`), `sprite.scale = 8/D`. 캐릭터의 `D = min(N, 16·S)`와 따로 판단하므로, 부모는 절차 생성이고 미니미만 커스텀인 조합도 창 크기 변화를 따라간다.
- 소유권: 커스텀 미니미 텍스처는 `CharacterEntity`가 소유하고 `replaceMinimi`/`destroy`에서 해제한다. 프리필터를 거치지 않은 패스스루 경로는 소스가 `minimiOverrides`의 공유 캔버스라 해제하지 않는다(`CharacterAssets.dispose`와 같은 규약).
- 라이브 반영: `appearanceKey`가 `minimiUpdatedAt`과 오버라이드 유무를 포함해 지정·해제 시 엔티티가 재생성된다. 트리거는 `App.tsx`가 넘기는 `resyncSignal`이다.

## 5. 결정성과 테스트

- 원칙: `gen/`의 순수 계층(`prng`·`palette`·`parts`·`compositor`·`archetypes`·`sheetGen`·`spriteResample`)은 Pixi/DOM 전역에 의존하지 않고 `CanvasFactory` 주입만 받는다. 텍스처화는 `characterFactory`·`minimiFactory`·`emojiTexture`·`awardPortraitTexture`에만 있다.
- 테스트 시임: `gen/__tests__/helpers.ts`가 `@napi-rs/canvas`로 `CanvasFactory`를 구현하고 `getImageData`로 픽셀을 비교한다. vitest 기본 환경은 `node`이고(`vitest.config.ts`), DOM이 필요한 파일만 `@vitest-environment jsdom`을 단다. `entities/__tests__/helpers.ts`는 렌더러 없이 만들 수 있는 실제 `Container`/`Sprite`/`Graphics`로 엔티티를 검증한다.
- 테스트 위치(심볼 단위로 고정하는 계약):

| 파일 | 고정하는 계약 |
|---|---|
| `gen/__tests__/characterFactory.test.ts`, `characterFactory.custom.test.ts` | 같은 시드 → 같은 픽셀, 다른 시드 → 다른 픽셀, 커스텀 시트 경로 |
| `gen/__tests__/archetypes.test.ts` | human 바이트 호환, 아키타입 추첨 결정성 |
| `gen/__tests__/palette.test.ts`, `colorOverrides.test.ts` | 셔츠-피부 대비 ≥ 1.6, 색 오버라이드가 rng 순서를 건드리지 않음 |
| `gen/__tests__/prng.test.ts`, `parts.test.ts`, `compositor.test.ts`, `spriteResample.test.ts`, `minimiFactory.test.ts`, `emojiTexture.test.ts`, `awardPortraitTexture.test.ts` | PRNG·16행 불변식·합성·프리필터·`MINIMI_CELL` |
| `map/__tests__/deskAssignment.test.ts`, `mapData.test.ts`, `TileRenderer.test.ts` | 순서 무관·충돌 없는 배정, 수동 지정 예약, 좌석·줄 슬롯 유도, 가구 분리 |
| `scenes/__tests__/scenes.test.ts`, `officeScene.test.ts` | 풍경 레지스트리 무결성(전 풍경 20×14, 8종 의미 타일 사용, 풍경×테마 전 조합 드로잉), `sceneColor` 변환 |
| `entities/__tests__/behaviorFsm.test.ts`, `CharacterEntity.test.ts`, 오버레이별 테스트 | FSM 전이표, 이동·도착·탕비실 예약, 오버레이 표시 조건 |
| `world/__tests__/OfficeWorld.test.ts`, `pathing.test.ts` | diff 동기화·외형 키·줄 순서·휴가·`setMap` |
| `__tests__/OfficeScene.test.ts`, `OfficeCanvas.test.tsx`, `bus.test.ts`, `awardee.test.ts` | 생명주기 가드, 카메라, bus 재생, 수상자 좌석 |

## 핵심 결정 요약

- 카메라: **고정 + 정수 스케일**(픽셀 선명도). 팬·줌 없음.
- 타일: 코드 드로잉. 바닥은 nearest로 한 장 베이크, 가구는 y-정렬 개별 Graphics. 그림은 풍경이 주입하는 `TileDrawFn`, 기계는 `TileRenderer` 하나.
- 풍경: 의미 타일 계약 하나 + 풍경별 GRID·드로잉. 좌석·줄·라운지는 GRID에서 유도.
- 캐릭터: 16×16, 문자 픽셀 배열 파츠, idle 2 + walk 2 프레임, nearest. 아키타입 8종, human은 원래 출력과 바이트 동일.
- 결정성: `seed → xfnv1a → mulberry32`가 팔레트·파츠·아키타입 추첨을, `agentId` 해시가 좌석·이동 난수를 정한다. 사용자 색은 rng 소비 뒤에 얹는다.
- 거동: 세션 활성·알림 대기면 자리, 아니면 탕비실. 알림 대기자는 휴가가 아니면 보스 책상 앞 줄에 선다.
- 경계: `OfficeBus` 하나와 `profiles` 전체 목록 prop으로 A/C와 격리한다.
