@echo off
setlocal
cd /d "%~dp0"
if not exist "target\release\ut99-rust.exe" (
    echo Build first: cargo +stable-x86_64-pc-windows-msvc build --release
    pause
    exit /b 1
)
if "%~1"=="" (
    "target\release\ut99-rust.exe" view "local\goty\Maps\DM-Deck16][.unr" 12
) else (
    "target\release\ut99-rust.exe" view "%~1"
)
if errorlevel 1 (
    pause
    exit /b 1
)
