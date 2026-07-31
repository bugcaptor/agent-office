# scripts

프로젝트 유지보수용 스크립트 모음.

## sign-macos.sh

macOS 빌드 산출물을 재서명해, macOS TCC(개인정보 보호) 권한 프롬프트가 매 빌드마다 다시 뜨는 것을 막는다.

### 배경

앱은 PTY로 `claude`·`codex` 같은 CLI 에이전트를 띄우는 터미널 호스트다. 자식 프로세스가 보호된 경로를 건드리면 macOS는 그 판정을 **책임 프로세스**(= 이 앱)에 귀속시켜 앱 이름으로 프롬프트를 띄운다. 여기까지는 정상 동작이라 없앨 수 없다.

문제는 **같은 프롬프트가 반복해서** 뜨는 것이었다. 서명이 애드혹이면 designated requirement가 cdhash 고정이라 빌드할 때마다 아이덴티티가 바뀌고, TCC에 저장된 허용/거부 결정이 통째로 무효화된다(`Failed to match existing code requirement`). 실제 인증서로 서명하면 이게 사라진다.

한 겹 더: Apple Development 인증서로 서명하면 codesign이 DR을 leaf의 `subject.CN`에 묶는데, CN에 든 인증서 ID가 재발급마다 바뀐다(무료 계정은 1년 만료). 그래서 이 스크립트는 DR을 **팀 ID(`subject.OU`)** 에 묶어 재발급에도 살아남게 한다.

### 사전 준비

1. Xcode > Settings > Accounts 에서 Apple ID 로그인 → Manage Certificates → `+` → **Apple Development** (무료 계정으로 충분)
2. `security find-identity -v -p codesigning` 이 `0 valid identities found` 이면 WWDR G3 중간 인증서가 없는 것이다:

   ```bash
   curl -fsSLO https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer
   security import AppleWWDRCAG3.cer -k ~/Library/Keychains/login.keychain-db
   ```

### 사용법

평소에는 npm 스크립트 하나면 된다. 재서명은 **빌드할 때마다** 필요하므로 빌드와 묶어놨다.

```bash
npm run build:mac      # 빌드 + 재서명
npm run install:mac    # 빌드 + 재서명 + /Applications 에 설치
```

스크립트를 직접 부를 수도 있다:

```bash
./scripts/sign-macos.sh                      # 기본 경로의 번들을 재서명
./scripts/sign-macos.sh --install            # 재서명 후 설치까지
./scripts/sign-macos.sh path/to/some.app     # 경로 직접 지정
```

환경변수로 덮어쓸 수 있는 것: `AGENT_OFFICE_TEAM_ID`(팀 ID), `APPLE_SIGNING_IDENTITY`(서명 아이덴티티), `AGENT_OFFICE_INSTALL_DIR`(설치 위치, 기본 `/Applications`).

`--install` 은 기존 설치본을 `rm -rf` 후 `ditto` 로 통째로 갈아끼운다(`cp -R` 은 옛 파일이 남는다). 앱이 실행 중이면 경고하지만 진행하며, 실행 중인 인스턴스는 옛 코드를 계속 쓰므로 종료 후 재실행해야 한다.

TCC 레코드 청소는 **처음 한 번만** 하면 된다. 애드혹 서명 시절의 깨진 항목을 지우는 용도라 이후 빌드에서는 불필요하다:

```bash
tccutil reset All com.bugcaptor.agent-office
```

### 참고

- 사진·미디어 라이브러리·이동식/네트워크 볼륨 프롬프트는 **거부해도 기능 손실이 없다**. 실제 접근 주체는 CLI 에이전트 샌드박스의 프리플라이트다.
- 전체 디스크 접근 권한(FDA)은 기본적으로 주지 않는다. 주는 순간 앱이 띄우는 모든 에이전트가 `~/Library` 전역에 접근하게 되므로, 폴더 단위 게이팅을 방어선으로 남긴다.
- 로그에 찍히는 `kTCCServiceScreenCapture` 는 WKWebView GPU 프로세스의 무프롬프트 프리플라이트라 무시해도 된다.
- `npm run build:mac` 은 `--bundles app` 으로 `.app` 만 만든다(DMG 생략 — 로컬 반복 빌드가 빠르다). 반면 `npm run tauri build` 는 `bundle.targets: "all"` 이라 DMG도 굽는데, **DMG 안의 앱은 재서명 전 상태**라 DR이 인증서 CN에 묶여 있다. 배포용 DMG를 만들 때는 재서명 이후 다시 굽거나, Developer ID 서명으로 넘어가야 한다.
- 이 과정 전체가 macOS 전용이다. TCC는 윈도우에 대응물이 없고, `bundle.macOS` 설정은 다른 플랫폼 빌드에서 무시된다.

## bump-version.mjs

버전이 들어있는 5개 파일을 한 번에 갱신한다:
`package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`.

### 사용법

```bash
node scripts/bump-version.mjs [major|minor|patch]   # 기본값: patch
node scripts/bump-version.mjs <x.y.z>               # 특정 버전 직접 지정
```

npm 스크립트로도 실행할 수 있다:

```bash
npm run bump          # patch
npm run bump:patch
npm run bump:minor
npm run bump:major
```

### 자리수 규칙

자리수를 올리면 아래 자리수는 0으로 리셋된다.

| 인자 | 예시 |
| --- | --- |
| `major` | `1.4.2` → `2.0.0` |
| `minor` | `1.4.2` → `1.5.0` |
| `patch` | `1.4.2` → `1.4.3` |

`x.y.z` 형식으로 직접 지정하면 계산 없이 그 버전으로 맞춘다 (예: `node scripts/bump-version.mjs 2.3.4`).

### 참고

- 현재 버전은 `package.json`의 `version`을 기준으로 읽는다.
- `Cargo.lock`은 `name = "agent-office"` 패키지 블록의 `version`만 교체하므로 의존성 버전은 건드리지 않는다.
- 스크립트는 버전만 바꾸며 커밋은 하지 않는다. 변경 후 직접 커밋하면 된다.
