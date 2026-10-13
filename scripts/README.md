# scripts

프로젝트 유지보수용 스크립트 모음.

## 풍경 미리보기 (render-scenes.mjs)

10개 풍경 × 4개 테마의 개별 PNG와 테마별 모음 이미지를 만든다. 실제 `drawTile`의 사각형·색을 CPU 캔버스에 그리며, 좌석에 생성 캐릭터를 올려 배경 위 가독성을 확인한다. 앱의 Pixi 캐시·y-sort·실시간 오버레이까지 재현하는 도구는 아니다.

```bash
rtk node scripts/render-scenes.mjs /tmp/agent-office-scenes
rtk node scripts/render-scenes.mjs /tmp/agent-office-scenes-before --baseline
```

`--baseline`은 풍경 모듈을 Git `HEAD`에서 읽어 커밋 전 배경과 비교한다. 작업 파일은 변경하지 않는다. 출력은 저장소 밖에 두고, 확인할 때는 개별 PNG를 정수 배율로 확대한다.

## 빌드 앤 런 (build-and-run.sh)

클론 직후 한 번에 실행하는 진입 스크립트(`npm start`로도 호출). 요구사항(node 18+·cargo) 점검 → 필요 시 `npm install` → 기본은 `npm run tauri dev`. `--install`은 macOS에서 자체 서명 인증서 확인(없으면 생성 여부를 물음) 후 `npm run install:mac`으로 서명 설치·실행하고, 그 외 OS는 `npm run tauri build` 후 산출물 경로를 안내한다.

```bash
./scripts/build-and-run.sh            # 바로 실행(개발 모드)
./scripts/build-and-run.sh --install  # 빌드·(macOS 서명)·설치
./scripts/build-and-run.sh --help
```

## macOS 서명 (make-signing-cert.sh, sign-macos.sh)

macOS TCC 권한 프롬프트(사진·미디어 라이브러리·이동식 볼륨)가 **빌드할 때마다** 다시 뜨는 것을 막는다. 배경과 원리는 [docs/macos-signing.md](../docs/macos-signing.md)가 정본이고, 여기서는 조작법만 다룬다.

한 줄 요약: 서명이 없으면(애드혹) designated requirement가 cdhash 고정이라 빌드마다 바뀌고, TCC에 저장된 권한 결정이 그때마다 무효화된다. 안정적인 인증서로 서명하면 사라진다.

### 사용법

```bash
npm run cert:mac       # 자체 서명 인증서 생성 — 최초 1회
npm run install:mac    # 빌드 + 서명 + /Applications 에 설치
npm run build:mac      # 빌드 + 서명 (설치는 안 함, .app만)
npm run build:mac:dmg  # 배포용 DMG 빌드 (설치는 안 함) — 릴리스에 올릴 산출물

tccutil reset All com.bugcaptor.agent-office   # 최초 1회, 애드혹 시절 잔재 청소
```

스크립트를 직접 부를 수도 있다:

```bash
./scripts/sign-macos.sh                      # 기본 경로의 번들에 서명
./scripts/sign-macos.sh --install            # 서명 후 설치까지
./scripts/sign-macos.sh path/to/some.app     # 경로 직접 지정
./scripts/sign-macos.sh --help
```

### 아이덴티티 선택 순서

`sign-macos.sh`는 이 순서로 서명 인증서를 고른다:

1. 환경변수 `APPLE_SIGNING_IDENTITY`
2. 자체 서명 `Agent Office Local Signing` (`make-signing-cert.sh`가 만든 것)
3. `Apple Development` (Xcode 무료 인증서)

인증서 종류마다 codesign이 만드는 자동 DR이 달라서, **Apple Development일 때만** DR을 팀 ID(`subject.OU`) 기준으로 덮어쓴다. 자체 서명과 Developer ID는 자동 DR이 이미 안정적이라 건드리지 않는다. 표는 정본 문서 §3 참고.

인증서는 이름이 아니라 **SHA-1 해시로 지정**해 서명하므로, 키체인 신뢰 설정이 없어도 동작한다.

### 환경변수

| 변수 | 용도 | 기본값 |
| --- | --- | --- |
| `AGENT_OFFICE_SIGNING_CN` | 자체 서명 인증서 이름 | `Agent Office Local Signing` |
| `AGENT_OFFICE_CERT_DAYS` | 인증서 유효기간(일) | `7300` (20년) |
| `AGENT_OFFICE_KEYCHAIN` | 인증서를 넣을 키체인 | 로그인 키체인 |
| `AGENT_OFFICE_SKIP_TRUST` | `1`이면 신뢰 설정 건너뜀 | `0` |
| `AGENT_OFFICE_INSTALL_DIR` | `--install` 설치 위치 | `/Applications` |
| `AGENT_OFFICE_TEAM_ID` | Apple Development용 팀 ID | 인증서 OU에서 자동 추출 |
| `APPLE_SIGNING_IDENTITY` | 서명 아이덴티티 직접 지정 | (자동 선택) |

### 참고

- `--install`은 기존 설치본을 `rm -rf` 후 **`ditto`** 로 갈아끼운다. `cp -R`은 확장속성을 흘려 서명을 깨뜨린다. 앱이 실행 중이면 경고하지만 진행하며, 실행 중인 인스턴스는 옛 코드를 계속 쓰므로 종료 후 재실행해야 한다.
- `npm run build:mac`은 `--bundles app`으로 `.app`만 만든다(DMG 생략 — 로컬 반복 빌드가 빠르다).
- **릴리스 DMG**(`npm run build:mac:dmg`)는 이름 기반 조회를 쓰므로 인증서에 신뢰 설정이 걸려 있어야 한다. universal(Intel+Apple Silicon) 빌드가 필요하면 `npm run build:mac:dmg -- --target universal-apple-darwin`. 아키텍처 선택·굽고 나서 확인·릴리스 노트 문구까지 전체 절차는 정본 문서 §6 참고.
- 인증서를 잃으면 DR이 바뀌어 사용자가 프롬프트에 한 번 더 답하게 된다. 현재 방침은 **백업 대신 재발급**이며, 그 근거와 백업으로 전환할 조건은 정본 문서 §5에 있다.
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
