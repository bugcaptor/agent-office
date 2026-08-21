#!/usr/bin/env bash
#
# 클론 직후 바로 빌드해서 실행해 보기 위한 스크립트 (macOS / Linux).
#
#   ./scripts/build-and-run.sh             # 개발 모드로 실행 (npm run tauri dev)
#   ./scripts/build-and-run.sh --install   # 빌드해서 설치 (macOS 는 서명까지)
#
# --install 을 macOS 에서 쓰면 자체 서명 인증서부터 확인한다. 서명 없이 빌드하면
# designated requirement 가 빌드마다 바뀌어 사진·미디어 라이브러리·이동식 볼륨
# 권한 프롬프트가 계속 다시 뜬다. 배경은 docs/macos-signing.md 가 정본이다.
#
# Windows 는 이 스크립트 대신 `npm install && npm run tauri dev` 를 쓴다.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SELF_CN="${AGENT_OFFICE_SIGNING_CN:-Agent Office Local Signing}"
NODE_MIN=18
OS="$(uname -s)"

DO_INSTALL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --install) DO_INSTALL=1 ;;
    -h|--help)
      cat <<USAGE
사용법: $0 [--install]

  (인자 없음)   의존성을 갖춘 뒤 \`npm run tauri dev\` 로 바로 실행한다.
  --install     macOS: 자체 서명 인증서 확인 -> 빌드 + 서명 + /Applications 설치 -> 실행
                그 외:  \`npm run tauri build\` 로 배포용 산출물을 만든다.
USAGE
      exit 0
      ;;
    *) echo "오류: 모르는 인자: $1" >&2; exit 1 ;;
  esac
  shift
done

# --- 요구사항 확인 -----------------------------------------------------------

require() { # require <명령> <이름> <설치 안내 URL>
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "오류: $2 이(가) 없다. 설치한 뒤 다시 실행해라." >&2
    echo "      $3" >&2
    exit 1
  fi
}

require node "Node.js ${NODE_MIN}+" "https://nodejs.org"
require npm  "npm (Node.js 에 포함)" "https://nodejs.org"
require cargo "Rust 툴체인" "https://rustup.rs"

NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]')"
if [ "$NODE_MAJOR" -lt "$NODE_MIN" ]; then
  echo "오류: Node ${NODE_MIN}+ 가 필요하다 (현재 $(node -v))." >&2
  echo "      https://nodejs.org" >&2
  exit 1
fi

echo "==> node $(node -v) / $(cargo --version)"

if [ "$OS" = "Linux" ]; then
  echo "    Linux 는 Tauri v2 시스템 패키지(webkit2gtk 등)가 필요하다."
  echo "    빌드가 실패하면: https://v2.tauri.app/start/prerequisites/"
fi

# --- 의존성 ------------------------------------------------------------------

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

# --- 기본 동작: 개발 모드 실행 -----------------------------------------------

if [ "$DO_INSTALL" = "0" ]; then
  echo "==> npm run tauri dev"
  echo "    첫 빌드는 Rust 의존성 컴파일 때문에 몇 분 걸린다. 창이 뜰 때까지 기다려라."
  exec npm run tauri dev
fi

# --- --install ---------------------------------------------------------------

if [ "$OS" != "Darwin" ]; then
  echo "==> npm run tauri build"
  npm run tauri build
  echo
  echo "산출물: $ROOT/src-tauri/target/release/"
  echo "  번들(.deb / .AppImage 등)은 그 아래 bundle/ 에 있다."
  exit 0
fi

if security find-identity -p codesigning 2>/dev/null | grep -qF "$SELF_CN"; then
  echo "==> 서명 인증서 확인: \"$SELF_CN\""
else
  echo "==> 자체 서명 인증서 \"$SELF_CN\" 이(가) 없다."
  echo "    서명 없이 설치하면 사진·미디어 라이브러리·이동식 볼륨 권한 프롬프트가"
  echo "    빌드할 때마다 다시 뜬다 (docs/macos-signing.md)."
  echo "    Apple 계정은 필요 없고, 만드는 데 몇 초 걸린다."
  ANS="n"
  if [ -t 0 ]; then
    printf '    지금 만들까? (npm run cert:mac) [Y/n] '
    read -r ANS || ANS=""
    ANS="${ANS:-Y}"
  else
    echo "    (비대화 실행이라 생성을 건너뛴다)"
  fi
  case "$ANS" in
    [Nn]*) echo "==> 인증서 생성을 건너뛴다. 쓸 수 있는 아이덴티티가 없으면 서명 단계에서 멈춘다." ;;
    *) npm run cert:mac ;;
  esac
fi

echo "==> npm run install:mac (빌드 + 서명 + /Applications 설치)"
npm run install:mac

echo "==> open -a agent-office"
open -a agent-office
