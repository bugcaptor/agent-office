#!/usr/bin/env bash
#
# 코드 서명용 자체 서명(self-signed) 인증서를 만들어 로그인 키체인에 넣는다.
#
# 왜 필요한가: 서명이 없으면(= 애드혹) designated requirement 가 cdhash 고정이라
# **빌드할 때마다** 아이덴티티가 바뀐다. macOS TCC 는 권한 허용/거부를 DR 에 묶어
# 저장하므로, DR 이 바뀌면 결정이 통째로 무효화되고 사진·미디어 라이브러리·이동식
# 볼륨 접근 프롬프트가 처음부터 다시 뜬다. 배경은 docs/macos-signing.md 참고.
#
# 유효기간이 20년인 이유: 인증서를 재발급하면 DR 이 바뀌어 TCC 결정이 다시 깨진다.
# 갱신 이벤트 자체를 없애는 게 목적이다.
#
# 이 인증서는 Gatekeeper 를 만족시키지 못한다. 남에게 배포한 앱이 경고 없이 열리게
# 하려면 유료 Developer ID + 공증이 필요하다. 여기서 얻는 것은 "이 기계에서 TCC
# 결정이 유지되는 것"이다.

set -euo pipefail

CN="${AGENT_OFFICE_SIGNING_CN:-Agent Office Local Signing}"
DAYS="${AGENT_OFFICE_CERT_DAYS:-7300}" # 20년
KEYCHAIN="${AGENT_OFFICE_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"

# Homebrew 의 OpenSSL 3.x 가 만든 PKCS#12 는 Apple Security 프레임워크가 읽지
# 못한다("MAC verification failed"). 모든 macOS 에 있는 LibreSSL 을 절대경로로
# 고정해 그 함정을 피한다.
OPENSSL=/usr/bin/openssl

if [ ! -f "$KEYCHAIN" ]; then
  echo "오류: 키체인이 없다: $KEYCHAIN" >&2
  exit 1
fi

if security find-certificate -c "$CN" "$KEYCHAIN" >/dev/null 2>&1; then
  echo "이미 있다: \"$CN\""
  echo "다시 만들려면 키체인 접근.app 에서 해당 인증서를 먼저 지워라."
  echo "(주의: 지우고 다시 만들면 DR 이 바뀌어 TCC 프롬프트가 한 번 더 돈다.)"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "인증서 생성 중: \"$CN\" (유효기간 ${DAYS}일)"

# extendedKeyUsage=codeSigning 이 없으면 codesign 이 코드 서명 아이덴티티로
# 인식하지 않는다.
"$OPENSSL" req -x509 -newkey rsa:2048 -sha256 -days "$DAYS" -nodes \
  -keyout "$WORK/key.pem" -out "$WORK/cert.pem" \
  -subj "/CN=$CN/O=$CN/C=US" \
  -addext "basicConstraints=critical,CA:false" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" \
  2>/dev/null

# 빈 암호로 만든 PKCS#12 는 `security import` 가 거부한다. 일회용 암호를 쓴다.
P12_PW="import-$$-$RANDOM"
"$OPENSSL" pkcs12 -export -out "$WORK/bundle.p12" \
  -inkey "$WORK/key.pem" -in "$WORK/cert.pem" -passout "pass:$P12_PW" 2>/dev/null

# -T /usr/bin/codesign: 서명할 때마다 개인키 접근 승인 창이 뜨지 않도록
# 미리 접근을 허용한다.
security import "$WORK/bundle.p12" -k "$KEYCHAIN" -P "$P12_PW" \
  -T /usr/bin/codesign -T /usr/bin/security >/dev/null

echo "키체인에 넣었다: $KEYCHAIN"

# 신뢰 설정은 **선택**이다. sign-macos.sh 는 인증서를 해시로 지정해 서명하므로
# 신뢰 없이도 동작한다. 다만 신뢰를 걸어두면
#   - `security find-identity -v -p codesigning` 목록에 뜨고,
#   - `tauri build` 의 signingIdentity(이름 기반 조회)로도 쓸 수 있다(릴리스 DMG).
# 사용자 신뢰 도메인에 쓰는 작업이라 macOS 인증 창이 한 번 뜬다.
echo
if [ "${AGENT_OFFICE_SKIP_TRUST:-0}" = "1" ]; then
  echo "AGENT_OFFICE_SKIP_TRUST=1 -- 신뢰 설정을 건너뛴다."
elif security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$WORK/cert.pem" 2>/dev/null; then
  echo "신뢰 설정 완료."
else
  echo "신뢰 설정을 건너뛰었다(취소했거나 권한 부족)."
  echo "로컬 빌드 서명에는 지장이 없다. 릴리스 DMG 를 구우려면 키체인 접근.app 에서"
  echo "이 인증서를 열어 '코드 서명'을 '항상 신뢰'로 바꿔라."
fi

echo
echo "--- 확인 ---"
security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -E "$CN|identities found" || true

cat <<EOF

다음 단계:
  npm run install:mac                            # 빌드 + 서명 + 설치
  tccutil reset All com.bugcaptor.agent-office   # 처음 한 번만

백업 권장: 이 인증서의 개인키를 잃으면(키체인 초기화, 기계 교체) 새로 만들어야
하고, 그때 DR 이 바뀌어 TCC 프롬프트가 한 번 더 돈다. 키체인 접근.app 에서
"$CN" 을 .p12 로 내보내 보관해 두면 그 일을 피할 수 있다.
EOF
