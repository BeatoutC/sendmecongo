@echo off
rem sendmecongo —— Windows 构建（批处理版，不需要 PowerShell，不需要管理员权限）
rem
rem     tools\build-windows.cmd
rem
rem 与 build-windows.ps1 的区别：本脚本不指定 --target，直接用你已装好的 host 工具链，
rem 因此不需要额外 rustup target add，也不需要 VS Build Tools 之外的东西。
rem MSVC 和 GNU 两种工具链都能跑。
setlocal
chcp 65001 >nul
cd /d "%~dp0.."

echo == sendmecongo Windows 构建
echo.

echo [1/4] 工具链
for /f "tokens=2" %%v in ('rustc -vV 2^>nul ^| findstr /b "release:"') do set RUSTC_VER=%%v
for /f "tokens=2" %%h in ('rustc -vV 2^>nul ^| findstr /b "host:"') do set HOST=%%h
if "%RUSTC_VER%"=="" (
    echo     [错误] 找不到 rustc。请确认 Rust 已装且在 PATH 中。
    exit /b 1
)
echo     rustc %RUSTC_VER%   host=%HOST%
echo     版本要求 >= 1.89：raptorq 的 x86 SIMD 路径使用了 avx512 target feature，
echo     低于此版本在 x86_64 上编译不过（Apple Silicon 上看不出来）。
echo     rust-toolchain.toml 已固定 1.98.1，rustup 会自动取用。

echo.
echo [2/4] 构建 release
cargo build --release
if errorlevel 1 (
    echo.
    echo     [失败] 常见原因：
    echo       - MSVC 工具链缺 link.exe：装 VS Build Tools；或改用 GNU：
    echo           rustup default stable-x86_64-pc-windows-gnu
    echo       - rustc 版本低于 1.89
    echo       - 离线环境无法下载 rust-toolchain.toml 指定的工具链
    exit /b 1
)

echo.
echo [3/4] 收集产物
if not exist dist mkdir dist
copy /Y "target\release\sendmecongo-send.exe" dist\ >nul 2>nul
copy /Y "target\release\sendmecongo-recv.exe" dist\ >nul 2>nul
copy /Y "target\release\sendmecongo-bench.exe" dist\ >nul 2>nul

echo.
echo [4/4] 完成
for %%f in (dist\*.exe) do echo     %%~nxf   %%~zf bytes
echo.
echo   发送端，图形界面：dist\sendmecongo-send.exe
echo   发送端，命令行播放：dist\sendmecongo-send.exe --play --in 文件 --preset turbo60 --size 1600
echo   接收端（给隔离外的人，零依赖）：dist\sendmecongo-recv.exe
echo       把录像拖到它上面，或和录像放同目录双击，即可还原出原文件。
echo.
echo   首次构建 sendmecongo-recv 会编译 openh264 的 C++ 源码，耗时较长，属正常。
endlocal
