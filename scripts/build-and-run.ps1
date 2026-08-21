# 클론 직후 바로 빌드해서 실행해 보기 위한 스크립트 (Windows).
#
#   .\scripts\build-and-run.cmd              # 개발 모드로 실행 (npm run tauri dev)
#   .\scripts\build-and-run.cmd --install    # NSIS 인스톨러 빌드 -> 설치 -> 실행
#
# PowerShell에서 직접 실행하려면:
#   powershell -ExecutionPolicy Bypass -File scripts\build-and-run.ps1 [--install]
#
# --install 은 관리자 권한이 필요 없다. Tauri NSIS 기본값(현재 사용자 설치)으로
# %LOCALAPPDATA%\agent-office 에 조용히 설치한 뒤 앱을 실행한다.
# macOS / Linux 는 이 스크립트 대신 scripts/build-and-run.sh 를 쓴다.

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$NodeMin = 18
$DoInstall = $false

foreach ($arg in $args) {
  switch ($arg) {
    { $_ -in "--install", "-Install", "/install" } { $DoInstall = $true }
    { $_ -in "-h", "--help", "/?" } {
      Write-Host @'
사용법: build-and-run.ps1 [--install]

  (인자 없음)   의존성을 갖춘 뒤 npm run tauri dev 로 바로 실행한다.
  --install     NSIS 인스톨러(.exe)를 빌드해 현재 사용자 계정에 설치하고 실행한다.
'@
      exit 0
    }
    default { Write-Host "오류: 모르는 인자: $arg" -ForegroundColor Red; exit 1 }
  }
}

# --- 요구사항 확인 -----------------------------------------------------------

function Require-Cmd([string]$Cmd, [string]$Name, [string]$Url) {
  if (-not (Get-Command $Cmd -ErrorAction SilentlyContinue)) {
    Write-Host "오류: $Name 이(가) 없다. 설치한 뒤 다시 실행해라." -ForegroundColor Red
    Write-Host "      $Url" -ForegroundColor Red
    exit 1
  }
}

Require-Cmd node  "Node.js $NodeMin+"          "https://nodejs.org"
Require-Cmd npm   "npm (Node.js 에 포함)"       "https://nodejs.org"
Require-Cmd cargo "Rust 툴체인 (MSVC 타깃)"     "https://rustup.rs"

# node -p 에 따옴표를 넘기면 Windows PowerShell 이 벗겨 먹으니 node -v 를 직접 파싱한다.
$NodeMajor = [int](((node -v) -replace '^v', '').Split('.')[0])
if ($NodeMajor -lt $NodeMin) {
  Write-Host "오류: Node $NodeMin+ 가 필요하다 (현재 $(node -v))." -ForegroundColor Red
  Write-Host "      https://nodejs.org" -ForegroundColor Red
  exit 1
}

Write-Host "==> node $(node -v) / $(cargo --version)"
Write-Host "    Rust 는 MSVC 툴체인이 필요하다. 링크 오류가 나면 Visual Studio Build Tools"
Write-Host "    (C++ 데스크톱 워크로드)를 설치해라: https://v2.tauri.app/start/prerequisites/"

# --- 의존성 ------------------------------------------------------------------

# 설치된 트리의 기준시각: npm 이 설치할 때마다 쓰는 node_modules/.package-lock.json
# 이 가장 정확하다. 없으면(구형 npm 등) node_modules 디렉터리 자체로 비교한다.
$InstalledMark = "node_modules\.package-lock.json"
if (-not (Test-Path $InstalledMark)) { $InstalledMark = "node_modules" }

if (-not (Test-Path "node_modules")) {
  Write-Host "==> npm install (node_modules 없음)"
  npm install
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} elseif ((Test-Path "package-lock.json") -and
          ((Get-Item "package-lock.json").LastWriteTime -gt (Get-Item $InstalledMark).LastWriteTime)) {
  Write-Host "==> npm install (package-lock.json 이 설치된 트리보다 최신)"
  npm install
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} else {
  Write-Host "==> 의존성 최신 -- npm install 생략"
}

# --- 기본 동작: 개발 모드 실행 -----------------------------------------------

if (-not $DoInstall) {
  Write-Host "==> npm run tauri dev"
  Write-Host "    첫 빌드는 Rust 의존성 컴파일 때문에 몇 분 걸린다. 창이 뜰 때까지 기다려라."
  npm run tauri dev
  exit $LASTEXITCODE
}

# --- --install ---------------------------------------------------------------

Write-Host "==> npm run tauri -- build --bundles nsis (인스톨러 빌드)"
npm run tauri -- build --bundles nsis
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Setup = Get-ChildItem "src-tauri\target\release\bundle\nsis\*-setup.exe" -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $Setup) {
  Write-Host "오류: 인스톨러(.exe)를 찾지 못했다: src-tauri\target\release\bundle\nsis\" -ForegroundColor Red
  exit 1
}

# /S = 조용한 설치. Tauri NSIS 기본이 현재 사용자 설치라 관리자 권한이 필요 없다.
Write-Host "==> 설치: $($Setup.Name) /S"
Start-Process -FilePath $Setup.FullName -ArgumentList "/S" -Wait

# 설치 위치: 기본은 %LOCALAPPDATA%\agent-office, 아니면 레지스트리에서 찾는다.
$AppExe = Join-Path $env:LOCALAPPDATA "agent-office\agent-office.exe"
if (-not (Test-Path $AppExe)) {
  $Uninst = Get-ChildItem "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall" -ErrorAction SilentlyContinue |
    Get-ItemProperty -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -eq "agent-office" } | Select-Object -First 1
  if ($Uninst -and $Uninst.InstallLocation) {
    $AppExe = Join-Path $Uninst.InstallLocation "agent-office.exe"
  }
}

if (Test-Path $AppExe) {
  Write-Host "==> 실행: $AppExe"
  Start-Process -FilePath $AppExe
} else {
  Write-Host "설치는 끝났지만 실행 파일 위치를 찾지 못했다. 시작 메뉴에서 agent-office 를 실행해라."
  Write-Host "인스톨러: $($Setup.FullName)"
}
