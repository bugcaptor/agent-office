# tmux 세션 자동 호스팅 설계 (kbm #2pc)

상태: 정본 — 구현 완료(kbm #2pc). 갱신: 2026-08-30.

프로필에 "tmux 호스팅"을 켜두면, 그 캐릭터의 세션을 띄울 때 앱이 직접 tmux
세션을 새로 만들고 앱 PTY는 거기 붙기만 한다. 지금까지는 밖에서 tmux를 손수
만들고 `agent-office ctl attach <agentId> --tmux <target>`을 쳐야 했던 걸
자동화한 것이다(`ctl attach --tmux` 자체는 그대로 남는다 — 아래 §9).

## 1. 이 기능이 주는 것 — 그리고 안 주는 것

**주는 건 두 가지다.**

- 밖에서 같은 화면에 붙을 수 있다. iTerm이든 ssh로 들어온 다른 기기든
  `tmux attach`로 앱이 보고 있는 그 화면에 그대로 붙는다. sessiond는 앱이
  유일한 클라이언트라 구조상 이걸 못 준다.
- tmux를 tmux답게 쓸 수 있다. scrollback, pane 나누기, detach.

**안 주는 건 "앱을 껐다 켜도 하던 일이 이어진다"는 앱 재시작 내성이다.** 그건
지금도 앞으로도 sessiond 핸드오프(`docs/session-handoff-design.md`)가 맡는다.
둘은 겹치지 않는다 — 비호스팅 세션은 sessiond가 존속을 맡고, 호스팅 세션은
tmux가 존속을 맡는다.

## 2. 전제: 세션은 매번 새로 만든다

살아있던 tmux 세션에 다시 붙는 일은 하지 않는다. 소환할 때마다 새 이름으로
새 세션을 만든다.

이 전제 하나로 외부 세션 attach(`docs/external-session-attach-design.md`)를
설계할 때 제일 어려웠던 문제가 통째로 사라진다 — 재접속했을 때 옛 pane이 옛
sid로 훅을 계속 쏘고, `NotificationHub`가 그 sid를 등록 해제된 것으로 보고
이벤트를 폐기하는 문제다. "이미 있으면 붙기"(`new-session -A`)를 기각한
이유도 이것이다(§8).

## 3. 켰을 때 실제로 일어나는 일

프로필의 `tmuxHost?: boolean`가 켜져 있으면 세션을 띄울 때:

1. 그 캐릭터의 옛 세션을 정리한다(gc, §6).
2. 앱이 직접 `tmux new-session -d -s <이름> -c <cwd> -e K=V …`를 돌린다. 훅에
   필요한 env(`AGENT_OFFICE_SESSION`, `HOOK_URL`, `APP_DATA`, `SETTINGS`,
   `PI_EXT`)와 zsh 심 경로(`ZDOTDIR`)를 전부 `-e`로 박는다. 단 `TERM`은
   뺀다(§5).
3. 프로필에 시작 명령어가 있으면 `tmux send-keys`로 pane 안에서 실행한다.
   갓 만든 세션이라 빈 셸임이 보장돼서 안전하다.
4. 앱 PTY는 지금과 똑같이 `exec tmux attach-session -t '=<이름>'`으로 뜬다.

4번이 `ctl attach --tmux`가 이미 쓰던 바로 그 경로다. 그래서 출력
미러링·입력 주입·resize·`on_exit`·세션 로그·봇 inject가 전부 손 안 대고
그대로 동작한다(pipe-pane + send-keys로 별도 파이프라인을 새로 만드는 안은
가짜 SpawnedPty·이스케이프·크기 불일치가 딸려 와서 기각했다).

구현: `src-tauri/src/session/tmux_host.rs::ensure_hosted`.

## 4. 이름 규칙

```
ao-<작업 폴더 이름>-<시작 시각 HHMM>
```

예: `ao-agent-office-2134`. 이 이름의 목적은 하나다 — 사람이
`tmux list-sessions`를 눈으로 훑고 "어느 저장소에서 언제 띄운 것"을 바로
읽을 수 있어야 한다.

**캐릭터 이름은 이름에서 뺐다.** 슬러그가 캐릭터 이름에서 나오는데, 순한글
이름은 ASCII가 통째로 비어 전부 같은 폴백으로 몰린다. 실제 목록이
`ao-agent-adhgji-7ae87f`, `ao-agent-favfei-268b19`처럼 사람이 읽을 정보가
하나도 없는 줄로만 찼다. 그래서 캐릭터 이름 슬러그와 agentId 조각을 빼고,
그 자리에 작업 폴더 이름과 시각을 넣었다.

**그럼 gc는 무엇으로 "그 캐릭터 것"을 가리는가.** 이름이 아니라 세션에
직접 달아 둔 tmux 사용자 옵션 `@ao_agent`(값은 agentId)로 가린다(§9).
가리는 근거를 이름에서 떼어냈으니 이름은 사람이 읽기 좋게만 지으면 된다.

**같은 이름이 겹치면 뒤로 늘린다.** 같은 폴더에서 같은 분에 둘을 띄우면
이름이 겹친다. gc 직전에 받아 둔 `list-sessions` 목록에 그 이름이 이미
있으면 초까지 붙이고(`ao-agent-office-213456`), 그래도 겹치면 sid 앞 4자를
더 붙인다. 목록은 어차피 gc 때문에 받아 오므로 tmux 호출이 늘지 않는다.

슬러그 규칙: 소문자화 → `[a-z0-9]` 외 전부 `-` → 연속 `-` 압축 → 앞뒤 `-`
제거 → 최대 20자. 비면 `"work"`. tmux에서 문제가 되는 `.` `:`는 이 필터가
자동으로 걷어낸다.

프로필에서 고정 이름을 받지 않는다 — 충돌을 만들고, "이미 있으면 붙기"로
되돌아가고 싶어지게 만든다. **밖에서 이 세션을 찾을 때 기댈 수 있는 건
`ao-` 접두어뿐이고, 그게 사용자와 맺는 유일한 약속이다.** 나머지(폴더
이름·시각·중복 회피 꼬리)는 사람이 눈으로 읽으라고 있는 것이지 파싱해서
쓰라고 만든 형식이 아니다.

구현: `tmux_host::slug`, `tmux_host::dir_slug`, `tmux_host::session_name`.

## 5. `-t`는 `=` 정확일치를 쓴다

tmux `-t`는 접두/fnmatch 매칭이다 — `ao-agent-office-2134`가
`ao-agent-office-213456`을 잡을 수 있다. 중복 회피가 초를 덧붙이는 방식이라
(§4) 실제로 생길 수 있는 짝이다. gc·kill·attach가 전부 엉뚱한 세션을
건드릴 수 있는 자리다. 그래서 **우리가 만든 이름에는 전부 `-t '=<name>'`을
쓴다**(`tmux_host::attach_command_exact`, `kill_session_args`,
`send_keys_args`).

`ctl attach --tmux`는 사용자가 손으로 치는 이름이라 퍼지 매칭이 오히려
편의다. 그래서 기존 `control/tmux.rs::attach_command`(퍼지 매칭)는 그대로
두고, `tmux_host.rs`에 정확일치 버전을 별도로 뒀다. 둘은 이름은 비슷해도
용도가 다른 별개 함수다.

## 6. `-e`로 env를 박고, `TERM`은 뺀다

**왜 앱이 직접 subprocess를 돌리는가**(셸 래퍼가 아니라):

- env를 argv로 넘기니 여러 줄짜리 persona 프롬프트도 인용 지옥이 없다.
- `-e`는 그 세션에만 붙는다. `set-environment -g`처럼 서버 전역을 더럽히지
  않아서, 사용자가 따로 쓰던 무관한 tmux 세션에 우리 훅이나 래퍼가 새어
  들어가는 사고가 안 난다.
- 셸 stdin에 한 줄 주입하는 기존 계약과 안 싸운다.

**왜 "새로 만드니까 env를 그대로 물려받겠지"가 틀렸는가**: `tmux new-session`은
tmux 서버가 이미 떠 있으면 그 서버에서 pane을 포크한다 — 새 pane의 env는
클라이언트가 아니라 서버가 들고 있는 값에서 온다(`update-environment`
목록에 없는 변수는 안 넘어간다). 서버가 갓 뜬 경우조차 zsh 심이 `ZDOTDIR`을
원복한 뒤라 래퍼가 사라진다. 그래서 `-e`로 직접 박아야 한다.

`ZDOTDIR`은 새로 만들 필요가 없었다 — `shells::resolve_observed_with_shims`가
zsh일 때 이미 `ResolvedShell.extra_env`에 심 경로를 넣어 주고,
`create_with_profile`이 그 값을 쥐고 있다. 호스팅 분기는 `resolved.extra_env`를
그대로 `-e`로 옮기기만 하면 된다.

**왜 `TERM`은 뺀다**: `create_with_profile`이 끼워 넣는
`TERM=xterm-256color`는 앱 PTY(=tmux 클라이언트)용 값이다. pane은 tmux가
`tmux-256color`를 직접 정한다. 그대로 `-e`로 박으면 pane 안 프로그램이
잘못된 terminfo로 뜬다. pane env는 plan env + `resolved.extra_env`에서
`TERM`만 뺀 것이다(`tmux_host::pane_env`).

## 7. 세션 수명

원리는 하나다 — **의도적으로 끝낸 것은 tmux까지 죽이고, 크래시와 "유지하고
종료"만 남긴다.** 그러면 살아남는 세션은 전부 "밖에서 이어 쓰라고 남긴 것"이
된다.

| 트리거 | 결과 |
| --- | --- |
| `dispose`(탭 닫기, 캐릭터 세션 종료) | `tmux kill-session` — 일반 세션을 죽이는 것과 같은 뜻 |
| "다시 띄우기" | 죽이고 새 이름으로 다시 만든다(재접속 아님) |
| 종료 모달 "모두 종료하고 종료" | 호스팅 세션도 kill |
| 종료 모달 "유지하고 종료" | kill 안 함 — 아래 §8 |
| 앱 크래시 | 자연히 살아남는다 — 이게 이 기능의 값어치 그 자체 |
| pane에서 `Ctrl-b d` | 앱 탭은 `Exited`로 가지만 tmux 세션은 산다 — 디태치가 공짜로 생긴다 |

`dispose`의 kill 순서에 주의: **`tmux kill-session`을 먼저 해야** pane
프로세스가 SIGHUP을 받고, 그다음에 클라이언트 셸(앱 PTY)이 죽는다. 순서를
바꾸면 클라이언트만 죽고 tmux 세션이 고아로 남는다
(`manager.rs::dispose`).

## 8. "유지하고 종료"가 왜 특별 취급인지

설계 초안은 "호스팅 세션을 핸드오프 대상에서 빼기만 하면 된다"였는데, 코드를
실제로 읽어 보니 그걸로는 안 됐다.

`src-tauri/src/lib.rs`의 `RunEvent::ExitRequested` 핸들러는 **모달 선택과
무관하게 항상 `dispose_all()`을 부른다.** "유지하고 종료"가 지금 동작하는
이유는 `handoff_all`이 세션마다 `handed_off=true`를 세우고, `dispose()`가
그 플래그를 보면 즉시 return하기 때문이다(`manager.rs::dispose`). 그래서
호스팅 세션을 핸드오프에서 "빼기만" 하면 `handed_off`가 false로 남아
`dispose_all → dispose → tmux kill-session`이 그대로 돈다.

보정: 호스팅 세션도 **`handed_off=true`는 세우되 sessiond로의 fd 전송만
건너뛴다.** `handed_off`의 뜻을 "이 세션의 수명을 다른 주체가 가져갔다"로
확장한 것이고, 여기서는 tmux가 그 주체다(`cleanup_paths`도 안 지우므로 pane
안 claude도 계속 산다).

**이 마킹은 `app_data_dir` 확인·sessiond 접속 시도보다 반드시 앞에 있어야
한다.** `handoff_all`(`handoff_v1.rs`)은 `app_data_dir`가 없거나 sessiond
접속에 실패하면 세션을 하나도 안 건드리고 곧장 0을 리턴한다. 마킹이 그 뒤에
있으면, sessiond가 안 떠 있거나 접속에 실패하는 흔한 경우에 호스팅 세션까지
같이 버려진다 — 즉 sessiond 유무와 무관하게 tmux 호스팅 세션은 보존돼야
하므로, 마킹 코드는 그 조기 return보다 앞줄에 놓인다. 브로커 모드
경로(`handoff_broker.rs::handoff_all_broker`)도 같은 자리에 같은 처리가
있다 — v1은 브로커 모드면 이 함수로 위임하고 끝나므로, 브로커 모드에서
호스팅 세션을 보존하는 자리는 여기뿐이다.

## 9. gc — 소환 시점, 그 캐릭터 것만

`dispose`가 대부분을 죽이므로 보통은 안 쌓인다. 그래도 "유지하고 종료"를
반복하면 캐릭터당 하나씩 쌓인다. 그래서 **캐릭터를 소환할 때**
`@ao_agent` 값이 그 agentId이고 **붙어 있는 클라이언트가 없는** 세션만
죽인다(`tmux_host::ensure_hosted` → `orphans`).

- 소유자 표시는 `new-session` 바로 뒤에
  `set-option -t '=<name>' @ao_agent <agentId>` 한 번으로 단다. 실패해도
  세션은 죽이지 않는다 — 그 세션이 나중에 gc에 안 잡히고 남을 뿐이고,
  잘 뜬 세션을 소유자 표시 실패 하나로 죽이는 쪽이 더 나쁘다.
- 이 옵션이 없는 세션(사용자가 손으로 만든 것, 옛 버전이 만든 것)은 gc가
  건드리지 않는다. 이름이 우연히 `ao-`로 시작해도 마찬가지다.

- 부팅 시 전체 스캔이 아니라 소환 시점·그 캐릭터 한정이다. "매번 새 것이면
  된다"는 전제상 같은 캐릭터의 옛 세션은 정의상 대체된 것이라 죽여도 되고,
  다른 캐릭터 세션은 손대지 않으니 밖에서 쓰던 걸 뺏을 위험이 좁다.
- 붙어 있으면 안 건드린다 — 판정은
  `tmux list-sessions -F "#{session_attached}\t#{@ao_agent}\t#{session_name}"`
  한 번. 구분자가 공백이 아니라 탭이고 이름이 맨 뒤인 이유는, 밖에서 사람이
  만든 세션 이름에는 공백이 들어갈 수 있기 때문이다 — 앞 두 칸만 잘라내고
  나머지를 통째로 이름으로 받는다.
- 서버가 안 떠 있으면 tmux는 비영 종료 + stderr `no server running`을
  낸다. **이건 에러가 아니라 "고아 없음"으로 취급한다**(`ensure_hosted`).
- 살아남은 세션에 앱이 다시 붙는 일은 없다 — 앱을 다시 켜고 같은 캐릭터를
  소환하면(밖에서 안 붙어 있었다면) 그 옛 세션은 gc에 죽고 새 세션이 뜬다.
  밖에서 붙어 있던 세션은 살아남는다.

## 10. 강등 정책

tmux가 없거나(미설치·PATH 밖), 3.2 미만이거나(`-e` 옵션은 tmux 3.2부터),
세션 생성 자체가 실패하거나, Windows면 **세션 생성을 막지 않는다.** 일반
세션으로 그대로 뜨고, 터미널 맨 윗줄에 경고 한 줄이 붙는다(observer 강등
관례와 같다).

경고는 새 IPC 계약이나 배너 UI 없이 `install_session`의
`initial_output: Option<Vec<u8>>`으로 흘려보낸다 — 이 파라미터는 이미
리더 스레드가 뜨기 *전에* `ReaderMsg::Restore`로 먼저 나가므로(offset
회계에서도 제외), 터미널 맨 윗줄에 노란 경고 줄이 공짜로 붙는다
(`manager.rs::tmux_downgrade_notice`, `tmux_host_error_message`). 문구가
Rust 쪽 한국어 하드코딩이 되는 건 감수한다 — 한글 하드코딩 금지 테스트는
`src/renderer`·`src/shared`만 훑고, `control/routes.rs`에 이미 같은 선례가
있다.

버전은 `tmux -V`로 캐릭터 소환과 무관하게 딱 한 번만 확인한다
(`SessionManager.tmux_capability: OnceLock`).

## 11. 기각한 대안들

- **`new-session -A` + sid 영속(살아있는 세션이 있으면 붙기)**: 앱 재시작
  후에도 옛 pane의 claude 훅을 살리려는 안. 매번 새 세션이면 필요 없다.
  저장소·프로브 분기·`--resume`과의 상호작용 정의가 전부 딸려 오는데 값을
  안 한다.
- **tmux 3.2 미만용 폴백**(`set-environment -t`를 생성 직후에 쏘기): 첫
  pane에는 이미 늦다 — pane 셸은 세션 생성 시점에 이미 포크된다. 첫 pane이
  주 사용처인 기능에서 반쪽짜리라 만들지 않는다. pane 명령 인자로
  `env K=V… zsh`를 넘기는 안도 여러 줄 persona의 인용 지옥이라 함께
  기각했다.
- **`set-environment -g`로 서버 전역 주입**: 사용자가 따로 쓰던 무관한
  세션에 우리 값이 샌다. `-e`로 대체했다.
- **프로필 시작 명령어에 `exec tmux new-session …`을 쓰라고 문서로
  안내**: 제일 싼 안이지만 문제를 하나도 못 푼다. zsh 심(`ZDOTDIR`)이
  pane에 안 전해져서 `claude()` 래퍼가 없고, 그러면 claude가 `--settings`
  없이 떠서 훅이 아예 등록이 안 된다. 시작 명령어 자리도 이 안내를 위해
  뺏긴다.
- **부팅 시 전체 고아 세션 청소**: 붙은 클라이언트가 없다는 게 버려졌다는
  뜻이 아니다 — 안에서 claude가 아직 일하고 있을 수 있다. 소환 시점·그
  캐릭터 한정으로 좁혔다(§9).

## 12. 알려진 한계

1. **셸 작업중 감지가 안 뜬다.** `shell_activity.rs`는
   `tcgetpgrp(master) != 셸 pgid` + ICANON을 보는데, 호스팅 세션의 앱 PTY
   포그라운드는 영원히 raw 모드 `tmux attach-session`이다. pane에서
   `npm test`를 돌려도 캐릭터가 유휴로 보인다. 오탐이 아니라 **무탐**이라
   안전하고, 지금 `ctl attach --tmux`도 같은 성질이라 이 기능이 만든 새
   퇴화는 아니다.
2. pane 셸은 tmux `default-shell`이 정한다 — 프로필의 셸 선택이 무의미해져
   zsh 전용 한계(#73)를 그대로 물려받는다. bash 로그인 셸 사용자에게는 강등
   문구가 아니라 별개의 UI 힌트로 알려준다.
3. 살아있는 pane의 claude는 프로필을 고쳐도 반영되지 않는다 — 새로
   소환해야 새 프로필이 적용된다.
4. 여러 pane에서 동시에 CLI를 돌리면 상태가 섞인다(최근 이벤트가 이긴다).
   `#73`에서 이미 수용한 한계를 그대로 물려받는다.
5. 앱 재시작 후 옛 세션은 앱에서 안 보이고, 밖에서만 이어 쓴다. 논리
   세션(`external.rs`)과 달리 tmux 소유권 자체를 재현하는 입양 경로는
   만들지 않았다 — `adopt_one`/`adopt_one_broker` 둘 다 `hosted_tmux`에
   항상 `None`을 넘긴다.

부수적으로 얻은 것 하나: pane에서 `Ctrl-b d`로 디태치하면 앱 탭은 `Exited`로
가지만 tmux 세션은 산다. "탭은 닫되 밖에서 계속 쓰기"가 필요하면 이 우회로가
이미 있다 — 전용 디태치 버튼은 후속 과제다.

## 13. 어디서 소환해도 프로필을 따른다

`ctl create`(`control/routes.rs`)와 웹 원격 소환(`webremote/rpc.rs`)도 디스크
프로필의 `tmuxHost`를 읽는다. 웹쪽은 이미 `cwd`/`shell`/`startupCommand`를
전부 프로필에서 읽으므로 원래 일관적이었고, `ctl create`는 persona만 읽던
헬퍼를 확장해 `tmux_host`도 같이 읽는다.

`claude --resume`도 추가 분기가 필요 없다 — `sessionOptsFor`가 프로필의
나머지 필드는 유지한 채 `startupCommand`만 1회 교체하는 구조라,
`tmuxHost`만 실어 주면 리줌도 호스팅으로 뜨고 override된
`claude --resume <id>`가 그대로 `send-keys`로 pane에 들어간다.

## 14. 구현 맵

| 영역 | 파일 |
| --- | --- |
| 순수 로직 + 러너 주입 | `src-tauri/src/session/tmux_host.rs`(슬러그·이름·버전 파싱·세션 목록 파싱·argv 생성, `TmuxRunner` 클로저) |
| 매니저 배선 | `src-tauri/src/session/manager.rs`(`create_with_profile`의 호스팅 분기, `Session.hosted_tmux`, `install_session`, `dispose`) |
| 종료 수명 | `src-tauri/src/session/handoff_v1.rs::handoff_all`, `handoff_broker.rs::handoff_all_broker`(호스팅 세션 `handed_off` 조기 마킹) |
| 계약 | `src-tauri/src/types.rs`(`AgentProfile.tmux_host`, `CreateSessionRequest.tmux_host`), `src-tauri/src/ipc/commands/session.rs`(`SessionOpts.tmux_host`) |
| 원격 소환 | `src-tauri/src/control/routes.rs`(`ctl create`), `src-tauri/src/webremote/rpc.rs`(웹 원격) |
| `ctl attach --tmux`(별개, 무변경) | `src-tauri/src/control/tmux.rs` |
| 프런트 | `src/renderer/profile/sections/TerminalSection.tsx`, `renderer/profile/useProfileDraft.ts`, `renderer/ipc/sessionOpts.ts`, `shared/types/profile.ts`, `shared/types/session.ts`, `shared/i18n/locales/*/profile.json`(6개 로케일) |

## 15. #73과의 관계

`#73`("tmux 세션 지원 설계 — 상태 감지·로그 수집, env 전파 v1")이 문제의
절반을 이미 정리해 놨다. 승계한 것: 문제가 결국 env 전파로 환원된다는 틀,
`ZDOTDIR` 심이 안 전해지는 함정, 여러 pane에서 동시에 CLI를 돌리면 상태가
섞이는 것(최근 이벤트가 이김, 수용), zsh 전용 한계.

다만 **`#73` v1의 `tmux()` 셸 래퍼(`EnvBroadcastWrapperSpec`)는 이 기능의
선행 조건이 아니었다.** 여기서는 env를 앱 subprocess의 argv로 직접
넘기므로 래퍼 렌더러가 낄 자리가 없다. 이 기능이 생기면서 "앱 세션 안에서
사용자가 손으로 tmux를 띄우는" 주된 경우를 흡수하게 됐으므로, `#73` v1의
우선순위는 다시 따져볼 필요가 있다.
