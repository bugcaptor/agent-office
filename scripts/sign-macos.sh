#!/usr/bin/env bash
#
# macOS 앱 번들을 "TCC 결정이 유지되는" 서명으로 재서명한다.
# 배경과 원리는 docs/macos-signing.md 가 정본이다. 요약하면:
#
#   macOS TCC 는 권한 허용/거부를 그 앱의 designated requirement(DR)에 묶어
#   저장한다. 애드혹 서명(= 서명 설정을 안 했을 때의 기본값)은 DR 이 cdhash
#   고정이라 **빌드할 때마다** 바뀌고, 그때마다 저장된 결정이 무효화되어
#   사진·미디어 라이브러리·이동식 볼륨 프롬프트가 다시 뜬다.
#
# 인증서 종류마다 codesign 이 만드는 자동 DR 이 다르므로, 종류별로 다르게 다룬다:
#
#   자체 서명      -> `certificate root = H"<인증서 해시>"`
#                     인증서를 유지하는 한 안정적. 자동 DR 을 그대로 쓴다.
#   Developer ID   -> `... certificate leaf[subject.OU] = "<팀ID>"`
#                     이미 팀 기준이라 안정적. 자동 DR 을 그대로 쓴다.
#   Apple Development -> `... certificate leaf[subject.CN] = "Apple Development: 이름 (인증서ID)"`
#                     괄호 안 인증서 ID 가 재발급마다 바뀐다(무료 계정은 1년 만료).
#                     그래서 이 경우에만 DR 을 팀 ID(subject.OU)에 묶도록 덮어쓴다.

set -euo pipefail

BUNDLE_ID="com.bugcaptor.agent-office"
SELF_CN="${AGENT_OFFICE_SIGNING_CN:-Agent Office Local Signing}"
INSTALL_DIR="${AGENT_OFFICE_INSTALL_DIR:-/Applications}"

DO_INSTALL=0
APP=""

while [ $# -gt 0 ]; do
  case "$1" in
    --install) DO_INSTALL=1 ;;
    -h|--help)
      cat <<USAGE
사용법: $0 [--install] [앱_번들_경로]
  --install   재서명 후 $INSTALL_DIR 에 설치한다(기존 설치본을 교체).

서명 아이덴티티는 이 순서로 고른다:
  1) 환경변수 APPLE_SIGNING_IDENTITY
  2) 자체 서명 "$SELF_CN"   (scripts/make-signing-cert.sh 로 생성)
  3) "Apple Development"    (Xcode 무료 인증서)
USAGE
      exit 0
      ;;
    -*) echo "오류: 모르는 옵션: $1" >&2; exit 1 ;;
    *)  APP="$1" ;;
  esac
  shift
done

APP="${APP:-src-tauri/target/release/bundle/macos/agent-office.app}"

if [ ! -d "$APP" ]; then
  echo "오류: 앱 번들이 없다: $APP" >&2
  echo "먼저 \`npm run tauri build --bundles app\` 을 돌리거나, 경로를 인자로 넘겨라." >&2
  exit 1
fi

# -v 를 붙이지 않는다: 자체 서명 인증서에 신뢰 설정을 안 걸었어도 목록에 잡혀야
# 한다. 이름이 아니라 해시로 서명하므로 신뢰 여부와 무관하게 동작한다.
find_identity_hash() {
  security find-identity -p codesigning 2>/dev/null \
    | grep -F "$1" | head -1 | awk '{print $2}'
}

SIGN_ID=""
KIND=""
if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  SIGN_ID="$APPLE_SIGNING_IDENTITY"
  case "$SIGN_ID" in
    *"Apple Development"*) KIND="appledev" ;;
    *) KIND="explicit" ;;
  esac
elif SIGN_ID="$(find_identity_hash "$SELF_CN")" && [ -n "$SIGN_ID" ]; then
  KIND="selfsigned"
elif SIGN_ID="$(find_identity_hash "Apple Development")" && [ -n "$SIGN_ID" ]; then
  KIND="appledev"
else
  cat >&2 <<'ERR'
오류: 쓸 수 있는 코드 서명 아이덴티티가 없다.

자체 서명 인증서를 만들어라(권장, 무료·계정 불필요):
  ./scripts/make-signing-cert.sh

이미 만들었는데도 이 메시지가 나오면 아래로 확인해라:
  security find-identity -p codesigning
ERR
  exit 1
fi

# Apple Development 만 DR 을 덮어쓴다. 팀 ID 는 인증서 subject 의 OU 에서 읽는다.
REQ_ARGS=()
if [ "$KIND" = "appledev" ]; then
  TEAM_ID="${AGENT_OFFICE_TEAM_ID:-}"
  if [ -z "$TEAM_ID" ]; then
    TEAM_ID="$(security find-certificate -c "Apple Development" -p 2>/dev/null \
      | /usr/bin/openssl x509 -noout -subject 2>/dev/null \
      | tr ',/' '\n\n' | awk -F= '/^ *OU=/{gsub(/^ +/,"",$2); print $2; exit}')"
  fi
  if [ -z "$TEAM_ID" ]; then
    echo "오류: Apple Development 인증서에서 팀 ID(OU)를 못 읽었다." >&2
    echo "      AGENT_OFFICE_TEAM_ID 로 직접 넘겨라." >&2
    exit 1
  fi
  REQ_ARGS=(-r="designated => identifier \"$BUNDLE_ID\" and anchor apple generic and certificate leaf[subject.OU] = \"$TEAM_ID\"")
  echo "아이덴티티: Apple Development (팀 $TEAM_ID) -- DR 을 팀 ID 기준으로 덮어쓴다"
else
  echo "아이덴티티: $KIND ($SIGN_ID)"
fi

echo "서명 중: $APP"
# 중단된 codesign 이 남긴 .cstemp 가 있으면 다음 서명이
# "invalid or unsupported format for signature" 로 실패한다. 먼저 치운다.
find "$APP" -name "*.cstemp" -prune -exec rm -rf {} + 2>/dev/null || true
# `${A[@]+"${A[@]}"}`: bash 3.2(구형 macOS 기본 셸)에서 set -u 와 빈 배열이
# 만나면 unbound variable 로 죽는다. 그 조합을 피하는 관용구다.
codesign --force --sign "$SIGN_ID" ${REQ_ARGS[@]+"${REQ_ARGS[@]}"} "$APP"

echo
echo "--- 서명 확인 ---"
codesign -dv --verbose=2 "$APP" 2>&1 | grep -E "^(Identifier|Authority|TeamIdentifier|Signature)"

echo
echo "--- designated requirement ---"
codesign -d -r- "$APP" 2>&1 | grep designated

echo
codesign --verify --verbose=2 "$APP" 2>&1 | tail -2

if [ "$DO_INSTALL" = "1" ]; then
  DEST="$INSTALL_DIR/$(basename "$APP")"
  echo
  if pgrep -f "$DEST/Contents/MacOS/" >/dev/null 2>&1; then
    echo "경고: 앱이 실행 중이다. 교체해도 실행 중인 인스턴스는 옛 코드를 계속 쓰므로,"
    echo "      설치 후 반드시 종료하고 다시 실행해라."
  fi
  echo "설치 중: $DEST"
  # cp -R 은 확장속성을 흘려 서명을 깨뜨린다("code has no resources but signature
  # indicates they must be present"). 반드시 ditto 를 쓴다.
  rm -rf "$DEST"
  ditto "$APP" "$DEST"
  echo "설치 완료."
fi

cat <<EOF

TCC 레코드 청소는 **처음 한 번만** 하면 된다(애드혹 서명 시절의 깨진 항목 제거):
  tccutil reset All $BUNDLE_ID

그 뒤 앱을 실행하고 프롬프트에 서비스별로 한 번씩만 답한다.
사진 / 미디어 라이브러리 / 이동식·네트워크 볼륨은 거부해도 기능 손실이 없다
(실제 접근 주체는 앱이 띄운 CLI 에이전트의 샌드박스 프리플라이트다).

며칠 뒤 재발 여부 확인:
  /usr/bin/log show --last 1d --predicate 'process == "tccd"' --style compact \\
    | grep -i agent-office | grep "Failed to match"
아무것도 안 나오면 성공이다.
EOF
