# 독립 세션 서버와 데스크톱 원격 연결

상태: 구현. KBM [2w4](https://zgpdlx.tailc90d0d.ts.net:8444/t/2w4).

## 사용

서버는 macOS·Linux에서 실행한다. 데스크톱 앱의 데이터와 별도 디렉터리를 지정한다.

```sh
# 개발 빌드
cargo build --manifest-path src-tauri/Cargo.toml --bin agent-office
./src-tauri/target/debug/agent-office serve \
  --data-dir "$HOME/.agent-office-server" \
  --bind 127.0.0.1 --port 47373
```

패키지의 실행 파일에도 같은 `serve` 인자를 사용할 수 있다. GUI 부트스트랩 전에
분기하므로 서버에는 창이 뜨지 않는다. `--help`로 인자를 확인한다.

1. 서버 데이터 디렉터리의 `serve-token` 파일에서 접속 토큰을 복사한다. 서버는
   토큰 내용을 로그에 쓰지 않으며 파일 권한은 0600이다.
2. 다른 기기에서는 SSH 포트 포워딩을 열거나 서버를 Tailscale IP에 바인딩한다.

   ```sh
   ssh -N -L 47373:127.0.0.1:47373 user@server
   ```

3. 데스크톱 하단의 **원격 서버 연결** 버튼을 누른다. 별도 창에
   `http://127.0.0.1:47373`과 토큰을 입력한다. Tailscale 직접 연결은 서버의
   `http://100.x.x.x:47373`, HTTPS 프록시를 사용하면 `https://호스트:포트`를 입력한다.
4. 빈 서버에서는 원격 창에서 캐릭터를 만들고 서버의 작업 폴더 절대 경로와
   시작 명령어를 지정한 뒤 세션을 시작한다. 폴더 선택 대화상자는 클라이언트의
   파일시스템을 가리키므로 원격 창에서 제공하지 않는다.

기본 접속 정책은 loopback 및 tailnet이다. `--bind`는 리스너 주소를 정한다.
평문 HTTP는 SSH 터널·Tailscale 등 보호된 연결을 통해 사용한다. HTTPS의 인증서는
OS 신뢰 저장소로 검증하며 인증서 검증을 끄는 옵션은 없다.

클라이언트 창을 닫아도 서버 세션은 유지된다. 토큰은 원격 창의 localStorage나
URL에 저장하지 않는다. 서버가 SIGINT/SIGTERM을 받으면 저널을 비운 뒤 sessiond에
세션을 유지하고 종료한다. 같은 데이터 디렉터리로 다시 실행하면 세션을 입양한다.
서버 중복 실행은 파일 잠금으로 차단하며 비정상 종료 뒤에도 잠금 파일을 지울
필요가 없다.

## 지원 범위

- 같은 오피스·터미널 UI, 서버 프로필 작성·수정, 세션 시작·입력·크기 변경·종료.
- 활동·확인 요청 알림과 기존 초상 읽기, 원격 연결 상태 표시.
- 연결 단절 후 자동 재접속, 실행 중인 서버의 파일 저널에서 전체 또는 델타 복원.
- 프로필 저장의 revision 검사. 다른 클라이언트가 먼저 저장한 경우 기존 서버
  상태를 덮어쓰지 않고 충돌을 반환한다. 저장하지 못한 편집은 현재 창에 남는다.

봇·자동화 관리, 파일·Git 패널, OS에서 열기, 이미지 생성, 일기·로그 관리 및
앱 설정은 이 원격 모드에서 제공하지 않는다. 지원하지 않는 API를 로컬 API로
대체하지 않는다. 원격 창은 로컬 세션 입양, 주기 스냅샷 업로드, 요약기, 일기
작성기 및 마스코트 브리지를 설치하지 않는다.

**복원 보장의 경계:** 서버 프로세스가 계속 실행되는 동안 클라이언트가 끊겨도
처음부터 기록한 출력을 파일에서 재생한다. 서버 자체가 재시작되면 실행 프로세스는
sessiond로 유지되지만 화면은 기존 sessiond의 스냅샷·링 복원 범위에 제한된다.
이전 서버 실행의 저널은 보존하지만 새 스트림에 자동 병합하지 않는다. 서버가
중단된 동안 매우 많은 출력이 생긴 경우까지 완전 복원한다고 약속하지 않는다.
Windows는 원격 클라이언트 코드 경로를 제공하지만 서버 실행은 Unix 전용이며,
Windows 실기 검증은 하지 않았다.

터미널 표시 스크롤백은 기존 데스크톱과 같은 5000행이다. 파일 저널 자체는
스크롤백 상한과 독립적이다. 저널의 자동 보존 기간/용량 정리는 아직 없으므로
오래된 실행의 저널 관리는 서버 운영자가 한다. 실행 중인 저널은 삭제하지 않는다.

## 구현 계약

- `remote_server.rs`: Tauri 없이 Tokio runtime을 구성한다. `BrokerPtyFactory`는
  strict 모드여서 브로커 실패 시 앱 내부 PTY로 조용히 폴백하지 않는다.
- `webremote`: 기존 인증/WS 계약을 재사용한다. `serve-owner` 토큰만
  `office.snapshot`, `office.saveState`, `office.shells`를 호출한다. 프로필 저장은
  revision 비교와 쓰기를 같은 mutex 안에서 실행한다.
- `webremote/journal.rs`: 전용 writer가 출력과 resize를 JSONL에 기록하고 나서
  broadcast한다. 파일 경계가 고정된 reader가 최대 약 256KiB/128레코드 단위로
  복원한다. 제한된 writer 큐는 느린 디스크에서 생산자에게 backpressure를 준다.
  파일 기록 실패는 복원 오류로 반환하며 누락된 저널을 정상 복원으로 취급하지 않는다.
- 첫 복원은 빈 전체 스냅샷과 80×24 초기 크기에서 출력·resize 순서를 재생한다.
  델타 복원은 기존 화면을 지우지 않는다. 재생 중 터미널 프로토콜 응답을 새 입력으로
  서버에 돌려보내지 않는다.
- `remote_client`: 한 WS task가 RPC, 터미널 채널, 재접속을 소유한다. 렌더 완료
  offset을 ACK로 받으며 렌더러에 쌓인 출력에도 상한을 둔다. 입력/변경 RPC는
  연결 실패 시 재전송하지 않는다. Hello의 instanceId와 attach의 lastSessionId가
  다른 서버 실행·세션에 이전 offset을 적용하는 것을 막는다. 복원별 식별자로
  이전 화면에서 늦게 도착한 ACK도 무효화한다.
- native 이벤트는 원격 창에만 `remote:*` 이름으로 전달한다. 로컬 Tauri 이벤트의
  전체 창 broadcast와 구분한다. 원격 창은 실패·종료 중에도 로컬 API로 돌아가지 않는다.
- 프런트는 agent별 프레임 큐로 restore/output/resize를 직렬 처리하고 수신 위치와
  렌더 완료 위치를 나눈다. 같은 세션의 델타 재접속에서는 이미 큐에 있는 출력을
  다시 쓰지 않으며, 전체 복원은 새 generation으로 이전 ACK를 무효화한다.

## 검사

```sh
npx tsc --noEmit
npx vitest run --dir src
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
cargo build --manifest-path src-tauri/Cargo.toml --bin agent-office
node scripts/remote-server-smoke.mjs
```

마지막 검사는 전역 WebSocket을 지원하는 Node.js와 `python3`가 필요하며,
임시 데이터·토큰·실제 PTY만 사용한다. 인증 실패, 프로필 충돌,
1MiB 초과 전체 복원, 단절 중 출력, resize, 서버 재시작의 토큰·프로세스 유지까지
검증하고 테스트 서버를 종료한다. 설치된 데스크톱 앱은 교체하지 않는다.
