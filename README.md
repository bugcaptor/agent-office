# Agent Office

여러 코딩 에이전트 터미널 세션을 **2D 픽셀 아트 사무실**로 관리하는 데스크톱 앱입니다.

Claude Code 같은 에이전트를 여러 개 돌리면 어느 터미널이 무슨 일을 하고 있었는지 놓치기 쉽습니다. Agent Office는 세션 하나를 사무실에서 일하는 캐릭터로 보여줘, 타이쿤 게임 구경하듯 에이전트들을 한눈에 파악하고 오갈 수 있게 합니다.

## Quick Start

소스를 받아 직접 빌드해 쓰는 것이 기본 경로입니다.

**요구사항**

- [Node 18+](https://nodejs.org)
- [Rust 툴체인](https://rustup.rs)
- Linux는 Tauri v2 시스템 의존성이 추가로 필요합니다 → [공식 사전 준비 문서](https://v2.tauri.app/start/prerequisites/)

**macOS / Linux**

```bash
git clone https://github.com/bugcaptor/agent-office.git
cd agent-office
./scripts/build-and-run.sh
```

의존성 설치까지 알아서 하고 개발 모드(`npm run tauri dev`)로 앱을 띄웁니다. 첫 빌드는 Rust 컴파일 때문에 몇 분 걸립니다. `npm start`로도 같은 스크립트가 돕니다.

계속 쓸 거라면 응용 프로그램으로 설치하세요.

```bash
./scripts/build-and-run.sh --install
```

macOS에서는 자체 서명 인증서(없으면 만들지 물어봅니다)로 서명해 빌드하고 `/Applications`에 설치한 뒤 실행합니다. Apple 계정은 필요 없습니다. 서명을 해야 하는 이유는, 서명이 없으면 "사진·미디어 라이브러리·이동식 볼륨" 권한 프롬프트가 **빌드할 때마다** 다시 뜨기 때문입니다. 그 프롬프트는 전부 거부해도 기능에 지장이 없고(앱이 아니라 앱이 띄운 CLI 에이전트의 파일 접근이 앱 이름으로 집계되는 것), 자세한 사정은 [docs/macos-signing.md](docs/macos-signing.md)에 있습니다. macOS가 아니면 `npm run tauri build`로 배포용 산출물(`src-tauri/target/release/`)을 만듭니다.

**Windows**

```bash
npm install
npm run tauri dev
```

<details>
<summary><b>또는 — macOS DMG를 받아 설치</b></summary>

<br>

DMG를 열어 `agent-office.app`을 **응용 프로그램** 폴더로 끌어다 놓습니다.

첫 실행 때 "확인되지 않은 개발자" 경고가 뜹니다. Apple 공증($99/년 유료 프로그램)을 받지 않은 앱이라 그렇습니다. **한 번만** 허용해 주세요.

> 앱을 실행 → 경고가 뜨면 닫기 → **시스템 설정 → 개인정보 보호 및 보안** → 아래로 스크롤 → **"확인 없이 열기"**

이후 뜨는 "사진", "미디어 라이브러리", "이동식 볼륨" 권한 요청은 **전부 거부해도 기능에 지장이 없습니다.**

</details>

## 첫 실행 / 첫 에이전트

처음 켜면 **동의 화면**이 뜹니다. Claude/Codex 연동을 켤지 묻는 것인데 **전부 꺼진 상태가 기본**이고 나중에 하단 ⚙ 설정에서 언제든 바꿀 수 있습니다. 잘 모르겠으면 그냥 넘어가세요 — 세션 관리·캐릭터·테마 같은 핵심 기능은 연동 없이도 전부 동작합니다.

캐릭터를 만들려면 화면 하단 왼쪽의 **`＋ New Agent`**를 누르고 **이름**(예: `builder`)과 **시작 폴더**를 넣은 뒤 저장합니다. 나머지는 선택 사항입니다. 저장하면 캐릭터가 사무실로 출근하고 터미널 세션이 바로 시작됩니다.

| 하고 싶은 것 | 방법 |
| --- | --- |
| 터미널 열기 | 캐릭터를 **클릭** |
| 각종 동작 (폴더 보기·재시작·일기·퇴근 등) | 상단 세션 탭을 **우클릭** ([메뉴 전체](docs/guide.md#터미널-탭-우클릭-메뉴)) |
| 캐릭터 더 만들기 | 다시 **`＋ New Agent`** |
| 퇴근시킨 캐릭터 부르기 | 하단 **`🏠 출근`** |
| 테마 바꾸기 | 하단 오른쪽 테마 버튼 (밝음·미드나이트·벚꽃) |

에이전트가 응답을 기다리면 캐릭터 머리 위에 느낌표가 뜨고 오른쪽에 알림이 쌓입니다. 알림을 클릭하면 그 터미널로 바로 갑니다.

> 💡 캐릭터 외형이 마음에 안 든다면 [예쁜 캐릭터를 쉽게 만드는 법](docs/guide.md#예쁜-캐릭터를-쉽게-만드는-법)을 참고하세요. 이미지 생성 AI로 만든 그림을 올릴 수 있습니다.

## 더 알아보기

- [docs/guide.md](docs/guide.md) — **사용 안내**. 주요 기능, Claude/Codex 연동, 봇 모드, 사용 팁, 빌드·테스트·환경변수
- [docs/macos-signing.md](docs/macos-signing.md) — macOS 코드 서명과 권한 프롬프트가 반복되는 이유
- [docs/cli-control-design.md](docs/cli-control-design.md) — `agent-office ctl`로 앱을 외부에서 조종하기
- [docs/bot-mode-design.md](docs/bot-mode-design.md) — 캐릭터를 Gitea 이슈 봇으로 돌리기 (실험 중)
- [docs/README.md](docs/README.md) — 전체 문서 인덱스

## 라이선스

MIT — [LICENSE](LICENSE) 참조. 번들 폰트(Galmuri11, Neo둥근모)는 각자 SIL Open Font License 1.1을 따릅니다(`src/renderer/styles/fonts/LICENSE-*.txt`).
