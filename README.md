# Agent Office

여러 개의 코딩 에이전트 터미널 세션을 **2D 픽셀 아트 사무실**로 관리하는 데스크톱 앱입니다.
Claude Code 같은 에이전트를 여러 개 돌리다 보면 어느 터미널이 무슨 일을 하고 있었는지
문맥 전환이 어려워집니다. Agent Office는 세션 하나하나를 사무실에서 일하는 캐릭터로
보여줘서, 타이쿤 게임을 구경하듯 에이전트들을 한눈에 파악하고 오갈 수 있게 합니다.

## 주요 기능

- **에이전트 1명 = 터미널 세션 1개** — 캐릭터를 클릭하면 터미널이 열리고, 닫아도
  세션은 사라지지 않고 보존됩니다(보이기만 토글).
- **알림** — 에이전트가 응답을 기다리면 캐릭터 머리 위에 느낌표가 뜨고 화면
  오른쪽에 티커 알림이 쌓입니다. 클릭해서 터미널을 열면 해당 알림이 지워집니다.
- **머리 위 작업 라벨** — 각 캐릭터 위에 프로젝트·목표·현재 작업이 두 줄로
  표시됩니다. 로컬 Claude 또는 Codex CLI로 요약할 수도 있습니다(옵트인).
- **세션 시간 집계** — 프롬프트 입력부터 응답 완료까지, 세션별 작업/대기 시간을
  패널로 보여줍니다(옵트인, CLI 훅 기반).
- **픽셀 아트 캐릭터** — 프로필을 만들면 기본 정보가 랜덤 생성되고(직접 수정 가능)
  시드 기반 픽셀 캐릭터가 자동 생성됩니다. 인간 외에 엘프·오크·로봇·슬라임 등
  아키타입 8종을 지원하며, 직접 그린 스프라이트 업로드(크롭·배경 투명화 포함)나
  PixelLab API 생성으로 교체할 수 있습니다.
- **사무실 연출** — 세션이 돌아가는 동안 캐릭터는 자리에 앉아 일하고, 한가해지면
  탕비실로 가서 쉽니다. 밝음·미드나이트·벚꽃 테마 3종을 제공합니다.

## Claude / Codex 연동 (옵트인)

선택적 연동 기능은 전부 기본 **꺼짐**입니다. 첫 실행 동의 화면이나 하단의
⚙ 설정에서 앱 전체 설정을 바꿀 수 있습니다.

- **작업 라벨 요약** — Claude Haiku 또는 Codex `gpt-5.4-mini`(reasoning
  `low`) 중 하나를 선택합니다. 프롬프트마다 목표/현재를 한 번에 요약하고,
  세션이 실제 작업을 시작해 첫 작업 정황(에이전트 내레이션·도구 활동)이 잡히면
  목표를 한 번 더 다듬습니다 — '이슈 40 해결' 같은 지시도 실제 목적으로
  구체화됩니다. 선택만 바꾸면 호출하지 않으며, 실패해도 다른 provider나 모델로
  재시도하지 않습니다. 호출은 선택한 로컬 CLI 계정의 구독 사용량 또는 크레딧을
  소비합니다.
- **에이전트 관찰 (알림·시간측정)** — 새로 만드는 터미널에 직접 실행되는
  `claude`와 `codex` 명령용 훅을 함께 준비합니다. `codex`,
  `codex resume --last`, 프로필의 `codex` 시작 명령도 같은 셸 래퍼를
  통과합니다. Agent Office 화면은 어느 provider인지 따로 표시하지 않습니다.

Agent Office는 사용자 전역 Claude/Codex 설정을 수정하지 않고 승인 결정을
대신하지 않습니다. Codex command hook은 Codex의 `/hooks` 화면에서 사용자가
정확한 정의를 직접 검토하고 신뢰해야 하며, Agent Office는 hook trust를
우회하거나 자동 승인하지 않습니다.

관찰 래퍼는 Windows PowerShell, PowerShell 7, Git Bash와 현재 ZDOTDIR shim을
사용하는 zsh를 지원합니다. WSL은 관찰 래퍼 대상이 아닙니다. Codex의 prompt,
permission request, stop과 서브에이전트 시작·종료를 관찰하며
`PostToolUse` heartbeat는 현재 Codex가 지원하는 tool 경로에서만 best-effort입니다.

설정 변경은 새로 만드는 터미널부터 적용됩니다. 연동이 실패하거나 꺼져 있어도
터미널 세션 관리·캐릭터·테마 등 나머지 기능은 계속 동작합니다.

## CLI로 조종하기 (옵트인, 2단계 승인)

다른 AI나 스크립트가 실행 중인 Agent Office를 프로그래밍 방식으로 조종할 수
있습니다. 보안 표면이므로 **2단계 옵트인**입니다(기본 꺼짐).

1. ⚙ 설정 → **CLI 제어 (외부 조종)** 를 켠다 → 로컬(`127.0.0.1`) 제어 서버가
   임의 포트로 뜨고 `<app_data>/control-port`가 기록됩니다.
2. 같은 설정에서 **CLI 제어 승인**을 눌러 토큰을 발급한다 → 이때만
   `<app_data>/control-token`(0600)이 생기고, 그 전에는 서버가 떠 있어도 모든
   요청이 401입니다. 승인은 지속되며 **승인 취소**로 언제든 폐기할 수 있습니다.

이용하는 쪽은 **주소·포트·토큰을 알 필요가 없습니다.** 같은 바이너리의 `ctl`
서브커맨드가 `AGENT_OFFICE_APP_DATA`(세션 터미널엔 앱이 자동 주입) 또는 OS별
표준 app_data 경로에서 포트·토큰을 자동으로 찾습니다. `ctl`은 GUI를 띄우지
않는 단명 클라이언트로, 요청 1건을 보내고 종료합니다(중복 앱 실행 아님).

```bash
agent-office ctl status                      # 연결/승인 상태 점검
agent-office ctl list                        # 프로필 + 실행 중 세션
agent-office ctl create reviewer --cwd ~/proj
agent-office ctl send builder "npm test" --enter
agent-office ctl notifications builder --json
agent-office ctl dispose reviewer
agent-office ctl settings set soundEnabled=false
```

HTTP는 내부 전송 계층일 뿐 사용자·에이전트가 보는 인터페이스는 CLI입니다.
설계·위협모델·전체 명령/종료 코드는 `docs/cli-control-design.md`를 참고하세요.

## 사용 팁

### 터미널 탭 우클릭 메뉴

캐릭터를 만든 뒤 클릭하면 터미널이 열리고, 상단에 세션 탭이 줄지어 나타납니다.
이 **탭을 우클릭**하면 자주 쓰는 동작이 메뉴로 뜹니다.

- **터미널 재시작** — 해당 터미널 셸/세션을 다시 만듭니다. 이후 어떤 CLI가
  실행되는지는 프로필의 시작 명령어가 결정합니다.
- **VS Code로 열기** — 프로필의 시작 폴더(cwd)를 VS Code로 엽니다(cwd가 있어야 활성화).
- **프로필 편집** — 이름·아키타입·외형·시작 명령어 등을 수정합니다.
- **퇴근** — 세션을 종료하고 캐릭터를 사무실에서 내보냅니다(나중에 다시 소환 가능).
- **캐릭터 삭제** — 세션과 프로필을 완전히 제거합니다.

### 예쁜 캐릭터를 쉽게 만드는 법 (프롬프트 복사)

시드 기반 자동 생성 스프라이트도 쓸 수 있지만, **프롬프트를 복사해 이미지 생성 AI로
만든 그림을 올리는 것**이 원하는 외형의 캐릭터를 가장 쉽게 얻는 방법입니다.

1. 탭 우클릭 → **프로필 편집**을 엽니다.
2. **외모 힌트**·**픽셀아트 의뢰 문구**에 원하는 외형을 적습니다(예: "빨간 망토를 두른 마법사").
3. 외형 섹션의 **픽셀아트 프롬프트 복사**(스프라이트) 또는 **초상 프롬프트 복사**(초상화)
   버튼을 누르면 방금 입력한 힌트가 반영된 프롬프트가 클립보드에 복사됩니다.
4. 복사한 프롬프트를 원하는 이미지 생성 AI에 붙여넣어 그림을 만듭니다.
5. 만든 이미지를 **픽셀아트 업로드** / **이미지 업로드** 버튼으로 올리면 끝입니다
   (크롭·배경 투명화 지원). `PIXELLAB_API_KEY`가 있으면 **PixelLab로 생성**으로
   바로 만들 수도 있습니다.

> 업로드·생성 버튼은 **이미 만들어진 캐릭터의 프로필을 편집할 때** 나타납니다.
> 먼저 캐릭터를 만든 다음 프로필 편집에서 외형을 다듬으세요.

## 실행 방법

### 요구사항

- Node 18+
- Rust 툴체인 (`rustup` 미설치 시 [rustup.rs](https://rustup.rs)에서 설치)
- Tauri v2 기반이라 macOS/Windows/Linux를 모두 지원합니다.

### 개발 서버 실행

```bash
npm install && npm run tauri dev
```

### 릴리즈 빌드

```bash
npm install && npm run tauri build
```

빌드된 결과물은 `src-tauri/target/release/` 디렉터리에서 확인할 수 있습니다.
macOS의 경우 `.dmg`, Windows의 경우 `.msi` 또는 `.exe`, Linux의 경우 `.deb` / `.AppImage` 파일이 생성됩니다.

### 테스트

```bash
npm test -- --run
cargo test --manifest-path src-tauri/Cargo.toml
npm run typecheck
```

### 선택 환경변수

- `PIXELLAB_API_KEY` — 프로필 편집의 "PixelLab로 생성" 버튼으로 캐릭터 스프라이트를
  [PixelLab](https://www.pixellab.ai) API로 생성하려면 설정하세요. 없으면 해당
  버튼만 비활성화되고 나머지 기능은 영향이 없습니다.

### 폰트 참고

터미널/UI 폰트는 CDN 없이 로컬에 번들되어 있습니다(Galmuri11, Neo둥근모을 `DungGeunMo`로 대체). 자세한 내용은 `src/renderer/styles/fonts/README.md`를 참조하세요.

## 라이선스

MIT — [LICENSE](LICENSE) 참조. 번들 폰트(Galmuri11, Neo둥근모)는 각자
SIL Open Font License 1.1을 따릅니다
(`src/renderer/styles/fonts/LICENSE-*.txt`).
