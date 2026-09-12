#!/usr/bin/env bash
#
# 헤드리스 원격 세션 서버(`agent-office serve`)를 빌드해서 띄우는 운영 스크립트
# (macOS / Linux 전용 -- 서버 자체가 Unix 전용이다. Windows 는 지원하지 않는다).
#
#   ./scripts/serve.sh                       # release 빌드 후 기본 주소로 실행
#   ./scripts/serve.sh --debug               # debug 빌드로 실행 (반복 실행이 빠름)
#   ./scripts/serve.sh --no-build            # 빌드를 건너뛰고 기존 바이너리로 실행
#   ./scripts/serve.sh --bind 0.0.0.0 --port 47373
#   ./scripts/serve.sh --show-token          # 시작 후 토큰 값을 화면에 출력
#
# 데이터 디렉터리(기본 ~/.agent-office-server)는 데스크톱 앱의 데이터 디렉터리와
# 반드시 분리한다. 같은 상태(프로필·세션·저널)를 두 프로세스가 동시에 소유하면
# 서로 덮어쓴다. 자세한 사용법과 구현 계약은 docs/remote-server-design.md 가 정본이다.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

OS="$(uname -s)"
case "$OS" in
  Linux | Darwin) ;;
  *)
    echo "오류: agent-office serve 는 Unix 전용이다 (현재 OS: $OS). Windows 는 지원하지 않는다." >&2
    exit 1
    ;;
esac

DATA_DIR="${AGENT_OFFICE_SERVE_DATA_DIR:-$HOME/.agent-office-server}"
BIND="127.0.0.1"
PORT="47373"
DO_BUILD=1
PROFILE="release"
SHOW_TOKEN=0

usage() {
  cat <<USAGE
사용법: $0 [옵션]

  --data-dir <dir>  서버 데이터 디렉터리 (기본: \$AGENT_OFFICE_SERVE_DATA_DIR
                    또는 ~/.agent-office-server). 데스크톱 앱의 데이터 디렉터리와
                    반드시 분리해라 -- 같은 상태를 두 프로세스가 소유하면
                    프로필·세션·저널을 서로 덮어쓴다.
  --bind <ip>       리스너 주소 (기본: 127.0.0.1)
  --port <n>        리스너 포트 (기본: 47373)
  --debug           release 대신 debug 프로파일로 빌드·실행한다
  --no-build        빌드를 건너뛰고 이미 있는 바이너리로 바로 실행한다
  --show-token      시작 후 접속 토큰 값을 화면에 출력한다 (기본은 파일 경로만 안내)
  -h, --help        이 도움말을 출력한다

기본은 release 빌드다. 첫 release 빌드는 몇 분 걸린다. 웹 클라이언트 자산은
release 빌드에 컴파일 시각 기준으로 내장되므로, 빌드 전에 npm run web:build 가
자동으로 실행된다.

평문 HTTP 이므로 --bind 를 loopback 밖으로 열면 SSH 터널이나 Tailscale 등
보호된 연결 뒤에서만 접속해라. 자세한 내용은 docs/remote-server-design.md.
USAGE
}

require_value() { # require_value <옵션명> <다음 인자 존재 여부>
  if [ -z "${2:-}" ]; then
    echo "오류: $1 에 값이 필요하다." >&2
    exit 1
  fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir)
      require_value "--data-dir" "${2:-}"
      DATA_DIR="$2"
      shift 2
      ;;
    --bind)
      require_value "--bind" "${2:-}"
      BIND="$2"
      shift 2
      ;;
    --port)
      require_value "--port" "${2:-}"
      PORT="$2"
      shift 2
      ;;
    --debug)
      PROFILE="debug"
      shift
      ;;
    --no-build)
      DO_BUILD=0
      shift
      ;;
    --show-token)
      SHOW_TOKEN=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "오류: 모르는 인자: $1" >&2
      exit 1
      ;;
  esac
done

# --- 요구사항 확인 -----------------------------------------------------------

require() { # require <명령> <이름> <설치 안내 URL>
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "오류: $2 이(가) 없다. 설치한 뒤 다시 실행해라." >&2
    echo "      $3" >&2
    exit 1
  fi
}

if [ "$DO_BUILD" = "1" ]; then
  require cargo "Rust 툴체인" "https://rustup.rs"
  require node "Node.js" "https://nodejs.org"
  require npm  "npm (Node.js 에 포함)" "https://nodejs.org"
fi

# AGENT_OFFICE_SERVE_BIN 은 테스트용 훅이다(가짜 바이너리로 바꿔치기). 도움말에는
# 올리지 않는다 -- 정상적인 사용에서는 프로필에 맞는 빌드 산출물을 그대로 쓴다.
BIN="${AGENT_OFFICE_SERVE_BIN:-$ROOT/src-tauri/target/$PROFILE/agent-office}"

# --- 빌드 --------------------------------------------------------------------

if [ "$DO_BUILD" = "1" ]; then
  # 설치된 트리의 기준시각: npm 이 설치할 때마다 쓰는 node_modules/.package-lock.json
  # 이 가장 정확하다. 없으면(구형 npm 등) node_modules 디렉터리 자체로 비교한다.
  INSTALLED_MARK=node_modules/.package-lock.json
  [ -e "$INSTALLED_MARK" ] || INSTALLED_MARK=node_modules

  if [ ! -d node_modules ]; then
    echo "==> npm install (node_modules 없음)"
    npm install
  elif [ -f package-lock.json ] && [ package-lock.json -nt "$INSTALLED_MARK" ]; then
    echo "==> npm install (package-lock.json 이 설치된 트리보다 최신)"
    npm install
  else
    echo "==> 의존성 최신 -- npm install 생략"
  fi

  echo "==> npm run web:build"
  npm run web:build

  echo "==> cargo build --manifest-path src-tauri/Cargo.toml --bin agent-office ($PROFILE)"
  if [ "$PROFILE" = "release" ]; then
    echo "    첫 release 빌드는 몇 분 걸린다."
    cargo build --manifest-path src-tauri/Cargo.toml --bin agent-office --release
  else
    cargo build --manifest-path src-tauri/Cargo.toml --bin agent-office
  fi
else
  if [ ! -x "$BIN" ]; then
    echo "오류: --no-build 인데 바이너리가 없다: $BIN" >&2
    echo "      먼저 --no-build 없이 실행해서 빌드하거나, 경로를 확인해라." >&2
    exit 1
  fi
fi

# --- 실행 --------------------------------------------------------------------

mkdir -p "$DATA_DIR"
TOKEN_FILE="$DATA_DIR/serve-token"
# 재실행이라 파일이 이미 있어도 지우지 않는다. 서버는 기존 serve-token 의 값을
# 그대로 다시 쓰므로(없을 때만 새로 만든다) 거기 적힌 값이 곧 이번 실행의 토큰이다.
# 지웠다가 서버가 뜨지 못하면 토큰만 잃는다.

echo "==> $BIN serve --data-dir $DATA_DIR --bind $BIND --port $PORT"
"$BIN" serve --data-dir "$DATA_DIR" --bind "$BIND" --port "$PORT" &
CHILD=$!
# 이제부터는 자식 종료코드를 직접 받아서 그대로 넘겨야 하므로 errexit 를 끈다.
set +e

CHILD_DONE=0
CHILD_STATUS=0
on_signal() {
  if [ "$CHILD_DONE" = "0" ]; then
    kill "$CHILD" 2>/dev/null || true
  fi
}
trap on_signal INT TERM

# serve-token 파일이 생길 때까지 최대 15초 폴링한다. 그사이 자식이 죽으면
# (중복 실행·포트 점유 등) 그 종료코드로 같이 종료해서 서버의 오류 메시지가
# 그대로 보이게 한다.
WAITED=0
while [ ! -f "$TOKEN_FILE" ]; do
  if ! kill -0 "$CHILD" 2>/dev/null; then
    wait "$CHILD" 2>/dev/null
    exit $?
  fi
  if [ "$WAITED" -ge 150 ]; then
    echo "오류: 15초 안에 토큰 파일이 생기지 않았다: $TOKEN_FILE" >&2
    kill "$CHILD" 2>/dev/null || true
    wait "$CHILD" 2>/dev/null || true
    exit 1
  fi
  sleep 0.1
  WAITED=$((WAITED + 1))
done

# 토큰 파일이 재실행 전부터 있었다면 폴링이 즉시 통과한다. 서버는 리스너를 먼저
# 묶고 토큰을 쓰므로 포트 점유·중복 실행 같은 기동 실패는 곧바로 드러난다 --
# 잠깐 두고 자식이 아직 살아 있는지 다시 확인한 다음에 "떴다"고 말한다.
sleep 0.5
if ! kill -0 "$CHILD" 2>/dev/null; then
  wait "$CHILD" 2>/dev/null
  exit $?
fi

DISPLAY_HOST="$BIND"
if [ "$BIND" = "0.0.0.0" ]; then
  DISPLAY_HOST="<호스트 주소>"
fi

echo
echo "==> 서버가 떴다: http://$DISPLAY_HOST:$PORT"
echo "    토큰 파일: $TOKEN_FILE"
if [ "$SHOW_TOKEN" = "1" ]; then
  echo "    토큰: $(cat "$TOKEN_FILE")"
fi
echo "    데스크톱 하단바의 \"원격 서버 연결\" 버튼에 위 URL 과 토큰을 입력해라."
echo "    평문 HTTP 이므로 SSH 터널(ssh -N -L $PORT:127.0.0.1:$PORT user@host)이나"
echo "    Tailscale 등 보호된 연결 뒤에서만 접속해라."
echo

# 신호를 받으면 trap 이 자식에게 같은 신호를 보내지만, bash 의 wait 은 자기 자신이
# 신호로 깨어난 것만으로 128+신호 번호를 반환하고 돌아온다 -- 자식이 저널을 flush
# 하고 실제로 끝나는 걸 기다린 게 아니다. 자식이 아직 살아 있으면 다시 wait 해서
# 진짜 종료코드를 받는다.
while :; do
  wait "$CHILD"
  CHILD_STATUS=$?
  kill -0 "$CHILD" 2>/dev/null || break
done
CHILD_DONE=1
exit "$CHILD_STATUS"
