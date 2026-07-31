#!/usr/bin/env bash
#
# macOS 빌드 산출물을 "TCC 결정이 유지되는" designated requirement로 재서명한다.
#
# 왜 재서명이 필요한가:
#   Tauri가 `bundle.macOS.signingIdentity`로 서명하면 codesign이 DR을 자동
#   생성하는데, Apple Development 인증서의 자동 DR은 leaf의 subject.CN을 핀한다:
#     certificate leaf[subject.CN] = "Apple Development: 이름 (367QTRA243)"
#   괄호 안 인증서 ID는 **재발급할 때마다 바뀐다**(무료 계정은 1년 만료).
#   macOS TCC는 허용/거부 결정을 DR에 묶어 저장하므로, DR이 바뀌면
#   `Failed to match existing code requirement` 로 결정이 무효화되고 사진·
#   미디어 라이브러리·이동식 볼륨 접근 프롬프트가 처음부터 다시 뜬다.
#
#   그래서 DR을 인증서 ID가 아니라 **팀 ID(subject.OU)** 에 묶는다. 팀 ID는
#   같은 Apple ID를 쓰는 한 인증서를 몇 번 재발급해도 그대로다.
#
# 애드혹 서명(서명 미지정 기본값)은 DR이 cdhash 고정이라 **매 빌드마다** 깨진다.
# 이 스크립트의 존재 이유가 그것이다.

set -euo pipefail

BUNDLE_ID="com.bugcaptor.agent-office"
# 팀 ID. `security find-identity -v -p codesigning` 로 얻은 인증서의 subject.OU.
TEAM_ID="${AGENT_OFFICE_TEAM_ID:-5P333WMUVE}"
# 서명 아이덴티티. 실명이 저장소에 남지 않도록 접두어 매칭을 쓴다
# (codesign은 유일하게 매칭되면 접두어만으로 찾아낸다).
IDENTITY="${APPLE_SIGNING_IDENTITY:-Apple Development}"

INSTALL_DIR="${AGENT_OFFICE_INSTALL_DIR:-/Applications}"
DO_INSTALL=0
APP=""

while [ $# -gt 0 ]; do
  case "$1" in
    --install) DO_INSTALL=1 ;;
    -h|--help)
      echo "사용법: $0 [--install] [앱_번들_경로]"
      echo "  --install   재서명 후 $INSTALL_DIR 에 설치한다(기존 설치본을 교체)."
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
  echo "먼저 \`npm run tauri build\` 를 돌리거나, 경로를 인자로 넘겨라." >&2
  exit 1
fi

if ! security find-identity -v -p codesigning | grep -q "$IDENTITY"; then
  echo "오류: 유효한 코드서명 아이덴티티가 없다: $IDENTITY" >&2
  echo "Xcode > Settings > Accounts > Manage Certificates 에서 Apple Development 인증서를 발급하고," >&2
  echo "체인이 끊겼다면 WWDR G3 중간 인증서를 설치해라:" >&2
  echo "  curl -fsSLO https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer" >&2
  echo "  security import AppleWWDRCAG3.cer -k ~/Library/Keychains/login.keychain-db" >&2
  exit 1
fi

REQUIREMENT="designated => identifier \"$BUNDLE_ID\" and anchor apple generic and certificate leaf[subject.OU] = \"$TEAM_ID\""

echo "서명 중: $APP"
codesign --force --sign "$IDENTITY" -r="$REQUIREMENT" --timestamp "$APP"

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
  # cp -R 은 기존 번들 위에 병합돼 옛 파일이 남을 수 있다. 통째로 갈아끼운다.
  rm -rf "$DEST"
  ditto "$APP" "$DEST"
  echo "설치 완료."
fi

cat <<EOF

TCC 레코드는 처음 한 번만 청소하면 된다(애드혹 서명 시절의 깨진 항목 제거):
  tccutil reset All $BUNDLE_ID

그 뒤 앱을 실행하고 프롬프트에 서비스별로 한 번씩만 답한다.
사진 / 미디어 라이브러리 / 이동식·네트워크 볼륨은 거부해도 기능 손실이 없다
(실제 접근 주체는 앱이 띄운 CLI 에이전트의 샌드박스 프리플라이트다).

며칠 뒤 재발 여부 확인:
  /usr/bin/log show --last 1d --predicate 'process == "tccd"' --style compact \\
    | grep -i agent-office | grep "Failed to match"
아무것도 안 나오면 성공이다.
EOF
