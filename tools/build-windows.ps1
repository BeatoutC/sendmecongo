# 在 Windows 上构建 sendmecongo-send.exe（单文件、免运行时）。
#
#   powershell -ExecutionPolicy Bypass -File tools\build-windows.ps1
#
# 前置：Rust（rustup）+ Visual Studio Build Tools（MSVC 工具链）。
# CRT 采用静态链接（见 .cargo/config.toml），产出的 exe 不需要安装 VC 运行库。

$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")

$target = "x86_64-pc-windows-msvc"
Write-Host "== 检查目标工具链 $target" -ForegroundColor Cyan
rustup target add $target 2>$null

Write-Host "== 构建（release，LTO）" -ForegroundColor Cyan
cargo build --release --target $target

$exe = "target\$target\release\sendmecongo-send.exe"
if (-not (Test-Path $exe)) { throw "构建产物不存在：$exe" }
$recv = "target\$target\release\sendmecongo-recv.exe"
if (-not (Test-Path $recv)) { throw "构建产物不存在：$recv" }

$dist = "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item $exe $dist -Force
Copy-Item $recv $dist -Force

$file = Get-Item "$dist\sendmecongo-send.exe"
$size = [math]::Round($file.Length / 1MB, 1)
$recvFile = Get-Item "$dist\sendmecongo-recv.exe"
$recvSize = [math]::Round($recvFile.Length / 1MB, 1)
Write-Host ""
Write-Host "== 完成" -ForegroundColor Green
Write-Host "   发送端（GUI + 播放器）：$($file.FullName)  ($size MB)"
Write-Host "   接收端（给隔离外的人）：$($recvFile.FullName)  ($recvSize MB)"
Write-Host ""
Write-Host "   双击 sendmecongo-send.exe 进入 GUI；也可直接用命令行："
Write-Host "   .\sendmecongo-send.exe --play --in <文件> --preset turbo60 --size 1600"
Write-Host "   接收方把录像拖到 sendmecongo-recv.exe 上即可还原，无需任何环境。"
