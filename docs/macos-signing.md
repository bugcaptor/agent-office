# macOS 코드 서명과 권한 프롬프트

macOS에서 agent-office를 쓰면 "사진", "미디어 라이브러리", "이동식 볼륨" 접근 권한 프롬프트가 **반복해서** 뜬다. 이 문서는 왜 그런지와 어떻게 없애는지를 정본으로 정리한다.

## 1. 프롬프트가 뜨는 이유 — 책임 프로세스 귀속

agent-office는 PTY로 `claude`·`codex` 같은 CLI 에이전트를 띄우는 **터미널 호스트**다. 앱 자신은 보호된 경로를 건드리지 않는다(`src-tauri/src/file_scan.rs`의 스캔 루트는 항상 작업 폴더이고 `follow_links(false)`다).

건드리는 건 자식 프로세스이고, macOS TCC는 그 판정을 **책임 프로세스(responsible process)** 에 귀속시킨다. 그게 앱이라서 앱 이름으로 프롬프트가 뜬다. unified log에서 실제로 확인된 구조:

```
accessing  = com.anthropic.claude-code
responsible= /Applications/agent-office.app/Contents/MacOS/agent-office   <- 프롬프트에 뜨는 이름
requesting = com.apple.sandboxd                                            <- 에이전트 샌드박스 프리플라이트
```

이건 macOS의 정상 동작이라 없앨 수 없다. 터미널 앱이라면 모두 겪는다.

**이 프롬프트들은 거부해도 기능 손실이 없다.** 실제로 사진첩이나 음악 라이브러리를 읽는 코드는 없고, CLI 에이전트 샌드박스가 초기화하며 TCC 상태를 미리 조회하는 것뿐이다.

로그에 보이는 `kTCCServiceScreenCapture`는 WKWebView GPU 프로세스의 **프롬프트 없는** 프리플라이트라 무시해도 된다. 코드베이스에 화면 캡처 사용처는 없다.

## 2. "반복해서" 뜨는 이유 — 서명이 불안정해서

macOS TCC는 허용/거부 결정을 그 앱의 **designated requirement(DR)** 에 묶어 저장한다. DR이 바뀌면 저장된 결정이 무효화되고 처음부터 다시 물어본다.

서명 설정을 하지 않으면 Tauri 빌드는 **애드혹(ad-hoc)** 서명이 되고, 그 DR은 이렇다:

```
designated => cdhash H"d6e2da587070ab237f63d2d9b94c529667f1ae2e"
```

cdhash는 바이너리 내용의 해시라 **빌드할 때마다 바뀐다.** 그래서 로그에 이런 게 계속 찍힌다:

```
Failed to match existing code requirement for subject com.bugcaptor.agent-office
AUTHREQ_PROMPTING: service=kTCCServicePhotos …
```

실제 로그에서 최근 빌드 4개가 서로 다른 아이덴티티(`agent_office-ac6dcca4…`, `-9d44da39…`, `-eba3d4dc…`, `-293c4305…`)로 관측됐다.

**해결책은 안정적인 인증서로 서명하는 것이다.** 그러면 DR이 빌드 간에 불변이 되고, 서비스당 한 번만 답하면 끝난다.

## 3. 인증서 종류별 DR

codesign이 자동 생성하는 DR은 인증서 종류마다 다르다. 이 차이가 실무에서 중요하다.

| 인증서 | 자동 생성 DR | 안정성 |
| --- | --- | --- |
| 애드혹 | `cdhash H"…"` | **빌드마다 깨짐** |
| 자체 서명 | `certificate root = H"<인증서 해시>"` | 인증서를 유지하는 한 안정 |
| Apple Development | `… certificate leaf[subject.CN] = "Apple Development: 이름 (인증서ID)"` | **재발급마다 깨짐** (무료 계정 1년 만료) |
| Developer ID | `… certificate leaf[subject.OU] = "<팀ID>"` | 재발급에도 안정 |

Apple Development의 CN에는 인증서 ID가 박혀 있고 이게 재발급마다 바뀐다. 그래서 `scripts/sign-macos.sh`는 **이 경우에만** DR을 팀 ID(`subject.OU`) 기준으로 덮어쓴다. 나머지 종류는 자동 DR이 이미 안정적이라 건드리지 않는다.

## 4. 채택한 방식 — 자체 서명

유료 Apple Developer Program($99/년) 없이 해결하기 위해 **자체 서명 인증서**를 쓴다. 유효기간을 20년으로 잡아 갱신 이벤트 자체를 없앤다.

```bash
./scripts/make-signing-cert.sh   # 한 번만
npm run install:mac              # 빌드 + 서명 + 설치
tccutil reset All com.bugcaptor.agent-office   # 한 번만, 애드혹 시절 잔재 청소
```

### 알아둘 점

- **신뢰 설정은 선택이다.** `sign-macos.sh`는 인증서를 이름이 아니라 **SHA-1 해시로 지정**해 서명하므로, 키체인 신뢰 설정이 없어도 동작한다. 신뢰를 걸면 `security find-identity -v -p codesigning` 목록에 뜨고, `tauri build`의 `signingIdentity`(이름 기반 조회)로도 쓸 수 있다.
- **인증서를 잃으면 DR이 바뀐다.** 새 인증서 = 새 DR = TCC 프롬프트가 한 번 더 돈다. 백업·복원·재발급은 §5 참고.
- **전체 디스크 접근 권한(FDA)은 주지 않는 것을 권한다.** 이 앱은 반자율 에이전트를 여러 개 상시 구동하는 호스트라, FDA를 주면 앱이 띄운 모든 에이전트가 `~/Library` 전역에 접근하게 된다. 폴더 단위 게이팅을 방어선으로 남긴다. 나중에 실제로 워크플로가 막히면 그때 체크박스로 열면 된다.

### 함정 모음 (직접 부딪힌 것들)

- **PKCS#12는 `/usr/bin/openssl`(LibreSSL)로 만들어야 한다.** Homebrew OpenSSL 3.x가 만든 것은 Apple Security 프레임워크가 못 읽는다(`MAC verification failed`).
- **PKCS#12에 빈 암호를 쓰면 `security import`가 거부한다.** 일회용 암호를 쓴다.
- **앱 번들 복사는 `ditto`로 해라.** `cp -R`은 확장속성을 흘려 서명을 깨뜨린다(`code has no resources but signature indicates they must be present`).
- **중단된 codesign은 `.cstemp`를 남긴다.** 남아 있으면 다음 서명이 `invalid or unsupported format for signature`로 실패한다. `sign-macos.sh`가 서명 전에 치운다.
- Xcode가 만든 Apple Development 인증서는 키체인 ACL에 codesign이 없어서 **서명할 때마다 승인 창이 뜬다.** `make-signing-cert.sh`는 `-T /usr/bin/codesign`으로 미리 허용해 이 문제를 피한다.

## 5. 인증서 관리 — 백업·복원·재발급

인증서의 SHA-1 해시가 곧 DR의 해시다. 실측으로 확인된 대응이다:

```
security find-identity 이 보여주는 해시:  FBD0A627833D0E1A0C52C0B9B7129E96CF896312
서명된 앱의 DR:  certificate root = H"fbd0a627833d0e1a0c52c0b9b7129e96cf896312"
```

즉 **인증서가 같으면 DR이 같고, 바뀌면 TCC 결정이 무효화된다.** 아래 절차는 전부 이 한 줄에서 파생된다.

### 현재 방침: 잃으면 재발급한다

개인키를 `.p12`로 보관하는 대신, **잃어버리면 그냥 다시 발급하는 쪽을 택했다.** 근거는 이렇다.

- 이 프로젝트의 배포 규모에서 재발급의 대가는 **기존 사용자가 권한 프롬프트에 한 번 더 답하는 것**뿐이다. 기능이 깨지거나 앱이 안 열리지는 않는다.
- 반면 개인키 파일을 오래 안전하게 보관하는 일에는 그 나름의 관리 비용과 유출 위험이 따른다.

재발급은 그냥 다시 만들면 된다. 만든 뒤에는 배포본을 새 인증서로 다시 굽는다.

```bash
# 키체인 접근.app 에서 기존 "Agent Office Local Signing" 을 먼저 지운 뒤
npm run cert:mac
```

**이 판단은 배포 규모가 커지면 뒤집어야 한다.** 사용자가 많아질수록 "전원이 한 번 더 답한다"의 총합이 커지므로, 그때는 아래 백업 절차로 전환한다.

### 백업해 두기로 했다면

내보내기 — 키체인 접근.app에서 인증서 우클릭 → **항목 내보내기…** → `.p12`. CLI로도 된다:

```bash
security export -k ~/Library/Keychains/login.keychain-db \
  -t identities -f pkcs12 -o agent-office-signing.p12
```

복원(새 기계, 키체인 초기화 후):

```bash
security import agent-office-signing.p12 \
  -k ~/Library/Keychains/login.keychain-db \
  -P '<내보낼 때 지정한 암호>' \
  -T /usr/bin/codesign -T /usr/bin/security
```

`-T /usr/bin/codesign`을 빠뜨리면 서명할 때마다 키체인 승인 창이 떠서 빌드가 멈춘다.

**신뢰 설정은 `.p12`에 들어 있지 않다.** 인증서가 아니라 키체인의 별도 신뢰 저장소에 살기 때문에, 복원 후 다시 걸어야 한다. 릴리스 DMG를 구우려면 필수다(§6 참고). 로컬 빌드만 할 거면 생략해도 된다.

```bash
openssl pkcs12 -in agent-office-signing.p12 -clcerts -nokeys -out cert.pem
security add-trusted-cert -r trustRoot -p codeSign \
  -k ~/Library/Keychains/login.keychain-db cert.pem
rm cert.pem
```

복원 검증 — 백업 전에 적어둔 해시와 대조한다. 같으면 기존 릴리스 사용자의 TCC 결정도 그대로 유지된다.

```bash
security find-identity -p codesigning | grep "Agent Office Local Signing"
```

## 6. 배포 — DMG 만들기

자체 서명은 **Gatekeeper를 만족시키지 못한다.** 유료 멤버십 없이는 우회로가 없다 — 공증(notarization)은 Apple Developer Program 회원에게만 열린 서비스다. 그래도 배포본을 자체 서명으로 굽는 것은 **애드혹보다 낫다.** 인증서를 고정하면 DR이 릴리스마다 동일해서 사용자가 업데이트할 때 권한을 다시 묻지 않는다. 애드혹으로 뿌리면 릴리스마다 다시 묻는다.

### 사전 확인

- **인증서에 신뢰 설정이 걸려 있어야 한다.** `tauri build`는 `sign-macos.sh`와 달리 인증서를 **이름으로** 찾기 때문이다. `security find-identity -v -p codesigning`(`-v` 주의)에 떠야 한다.
- **아키텍처를 정한다.** 기본 빌드는 호스트 아키텍처 전용이다. Apple Silicon 기계에서 구우면 arm64 전용이 나오고 **Intel Mac에서는 열리지 않는다.** 둘 다 받으려면 universal 빌드가 필요하다.

  ```bash
  rustup target add x86_64-apple-darwin      # 최초 1회
  ```

- **버전을 올린다.** `npm run bump` (patch) — `package.json`·`package-lock.json`·`Cargo.toml`·`Cargo.lock`·`tauri.conf.json` 5개 파일을 함께 갱신한다.

### 굽기

```bash
# Apple Silicon 전용
APPLE_SIGNING_IDENTITY="Agent Office Local Signing" npm run tauri build

# Intel + Apple Silicon 겸용
APPLE_SIGNING_IDENTITY="Agent Office Local Signing" \
  npm run tauri build -- --target universal-apple-darwin
```

산출물은 `src-tauri/target/release/bundle/dmg/`(universal 빌드는 `target/universal-apple-darwin/release/bundle/dmg/`)에 생긴다.

### 굽고 나서 확인

DMG를 마운트해 안의 앱을 검사한다. **DR이 `certificate root = H"…"` 형태여야 한다.** `cdhash H"…"`가 나오면 서명이 안 붙은 것이니(= 이름 조회 실패) 신뢰 설정부터 다시 본다.

```bash
hdiutil attach <파일>.dmg
codesign -dv --verbose=2 /Volumes/*/agent-office.app   # Authority 에 인증서 이름
codesign -d -r-           /Volumes/*/agent-office.app   # designated requirement
lipo -archs               /Volumes/*/agent-office.app/Contents/MacOS/agent-office
hdiutil detach /Volumes/*/
```

### 릴리스 노트에 반드시 넣을 것

공증 없는 앱이라 이 안내가 없으면 사용자가 "손상되었습니다"로 오해하고 지운다.

> 첫 실행 시 경고가 뜨면: **시스템 설정 → 개인정보 보호 및 보안** → 아래로 스크롤 → **"확인 없이 열기"**

macOS 15부터 우클릭→열기 우회가 막혀 이 경로만 남았다. 터미널을 선호하는 사용자에게는 `xattr -dr com.apple.quarantine /Applications/agent-office.app`.

arm64 전용으로 구웠다면 **"Apple Silicon 전용"** 임을 함께 명시한다.

### 소스를 받아 직접 빌드하는 사람

가장 깔끔하다. **로컬 빌드한 앱에는 검역(quarantine) 속성이 붙지 않아 Gatekeeper가 아예 개입하지 않는다.** 각자 `make-signing-cert.sh`로 자기 인증서를 만들면 TCC 문제도 이 문서대로 해결된다.

### 나중에 유료 멤버십으로 갈 경우

Developer ID Application 인증서의 자동 DR은 이미 팀 ID 기준이라 **재서명이 불필요하다.** `sign-macos.sh`를 건너뛰고 `tauri build` 하나로 끝난다. 공증은 Tauri가 자동 처리하며, 환경변수는 `APPLE_ID`·`APPLE_PASSWORD`(앱 암호)·`APPLE_TEAM_ID`, 또는 App Store Connect API 키(`APPLE_API_ISSUER`·`APPLE_API_KEY`·`APPLE_API_KEY_PATH`)를 쓴다.

이때 팀 ID가 개인 팀과 달라질 수 있는데, 그러면 DR이 바뀌므로 **전환 시점에 TCC 프롬프트가 한 번 더 돈다.**

## 7. 검증

며칠 써 본 뒤 재발 여부를 확인한다:

```bash
/usr/bin/log show --last 1d --predicate 'process == "tccd"' --style compact \
  | grep -i agent-office | grep "Failed to match"
```

아무것도 안 나오면 성공이다. (`log`는 zsh 빌트인과 충돌하므로 절대경로로 부른다.)

## 8. 다른 플랫폼

이 문서 전체가 macOS 전용이다. TCC는 윈도우·리눅스에 대응물이 없고, `bundle.macOS` 설정은 다른 플랫폼 빌드에서 무시된다. 윈도우에서 서명이 의미 있는 건 SmartScreen 경고 완화 쪽인데, 그건 EV/OV 코드 서명 인증서가 필요한 별개 트랙이다.
