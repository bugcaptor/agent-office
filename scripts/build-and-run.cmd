@echo off
rem Windows 진입점 — 실행 정책과 무관하게 build-and-run.ps1 을 실행한다.
rem   scripts\build-and-run.cmd            개발 모드 실행
rem   scripts\build-and-run.cmd --install  인스톨러 빌드 + 설치 + 실행
rem PowerShell 7(pwsh)이 있으면 그쪽을 쓴다. 없으면 Windows PowerShell 5.1 로 떨어진다.
set "PS=powershell"
where pwsh >nul 2>&1 && set "PS=pwsh"
"%PS%" -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-and-run.ps1" %*
exit /b %ERRORLEVEL%
