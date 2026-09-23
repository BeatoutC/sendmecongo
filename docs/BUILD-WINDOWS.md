# SendMeCongo Windows 构建指南 (BUILD-WINDOWS.md)

给在 Windows 上构建 `sendmecongo-*.exe` 的人或 AI Agent。目标：**pull 下来直接编译，不改任何代码**，并且产出的 exe 带正确的图标、不依赖运行库。

---

## 0. 最短路径

```cmd
git clone https://github.com/BeatoutC/sendmecongo.git
cd sendmecongo
tools\build-windows.cmd
```

产物落在 `dist\`：

```
dist\sendmecongo-send.exe     发送端（GUI + 全屏播放器）
dist\sendmecongo-recv.exe     接收端（给隔离外的人，零依赖）
dist\sendmecongo-bench.exe    开发测量工具，不发布
```

**不需要改任何文件**。图标 `crates\sendmecongo-send\app.ico` 与 `crates\sendmecongo-recv\app.ico`
已经随仓库提供，构建脚本是自足的。

---

## 1. 环境前提

| 项目 | 要求 |
|---|---|
| Rust | rustup 装好即可。`rust-toolchain.toml` 已固定 **1.98.1**，rustup 会自动取用 |
| **最低版本** | **1.89**。raptorq 的 x86 SIMD 路径用了 avx512 target feature，低于此版本在 x86_64 上编译不过（在 Apple Silicon 上反而看不出来） |
| 工具链 | 二选一，见下 |

**工具链二选一：**

- **MSVC**（默认）：需要 Visual Studio Build Tools（提供 `link.exe`）。
- **GNU / mingw**：目标机器没有管理员权限、装不了 VS Build Tools 时用这条：

  ```cmd
  rustup default stable-x86_64-pc-windows-gnu
  ```

两条路都配了**静态 CRT**（`.cargo\config.toml` 里的 `+crt-static`），产出的 exe 不需要安装 VC 运行库。
两种工具链的产物都能正常带图标 —— 差异见 §3.1。

---

## 2. 三种构建方式，选一个

| 命令 | 需要管理员 | 需要 PowerShell | 说明 |
|---|---|---|---|
| `tools\build-windows.cmd` | 否 | 否 | **推荐**。用 host 工具链，MSVC / GNU 都能跑，不指定 `--target` |
| `powershell -ExecutionPolicy Bypass -File tools\build-windows.ps1` | VS Build Tools | 是 | 强制 `x86_64-pc-windows-msvc`，**每次都会重跑图标生成**（见 §5 的注意事项） |
| `cargo build --release` | 否 | 否 | 最裸。产物在 `target\release\`，不会收集到 `dist\` |

---

## 3. 图标：两条独立通路，别只查一条

exe 上的图标有**两个**，由完全不同的机制提供。一个坏了，另一个不会替它兜底：

| 图标 | 出现在哪 | 由谁设置 |
|---|---|---|
| **文件图标** + exe 的版本信息块 | 资源管理器里的 exe、「属性 → 详细信息」 | `crates\*\build.rs` 调用 `winres`，`res.set_icon("app.ico")` |
| **窗口图标** | 标题栏、任务栏、Alt-Tab | 运行时 `WM_SETICON`，来自 `sendmecongo_ui::icon::window_icon(include_bytes!("../app.ico"))` |

### 3.1 `build.rs` 按「目标平台」判断 —— 不要改回去

两个 crate 的 `build.rs` 都是这样开头的：

```rust
if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
    return;
}
```

> ⚠️ **不要**把它改回 `#[cfg(target_os = "windows")]`。
>
> build script **永远由宿主编译并运行**，那个 cfg 回答的是"我是不是**在** Windows 上构建"，
> 而不是"我是不是**为** Windows 构建"。改回去之后，从 macOS / Linux 交叉编译到 Windows 时
> 整块会被编译掉 —— exe 照样能编出来，**就是没有图标，而且不报任何警告**。

> ⚠️ `winres` 必须留在**无条件**的 `[build-dependencies]` 下，
> **不要**挪进 `[target.'cfg(target_os = "windows")'.build-dependencies]`。
> 挪过去之后，`build.rs` 里对 `winres` 的引用在非 Windows 宿主上解析不了，
> **macOS 自己的构建会直接编译不过**。

工具链差异（两处都已在代码里处理，列出来是为了排查时心里有数）：

| 工具链 | `winres` 产出 | 处理方式 |
|---|---|---|
| GNU / mingw | `resource.o` | 直接作为 `cargo:rustc-link-arg` 交给链接器。**必须这么做**：GNU ld 对静态库是按需拉取的，纯资源对象不定义任何符号，会被整块丢掉且无警告 |
| MSVC | `resource.lib` | 走 `winres` 原本的路径。代码里的 `obj.exists()` 守卫在 MSVC 下不成立，因此不追加 link-arg |

### 3.2 exe 没有图标时，按这个顺序查

```cmd
rem 1) 资源段到底进没进二进制
objdump -h target\release\sendmecongo-send.exe | findstr rsrc
```

- **没有 `.rsrc`** → 资源确实没进去。依次检查：
  - `crates\sendmecongo-send\app.ico` 是否存在（它应当随仓库提供）；
  - 构建输出里有没有 `cargo:warning=...本次产物不会带图标` 这条警告 —— 有的话说明缺资源编译器：
    GNU 需要 `windres`（随 mingw 提供），MSVC 需要 `rc.exe`（随 VS Build Tools 提供）。
- **有 `.rsrc`，资源管理器里却是白图标** → 多半是 Windows 图标缓存。换个目录看，或注销重登。

### 3.3 验证图标时，**不要**用 `System.Drawing.Icon`

`[System.Drawing.Icon]::ExtractAssociatedIcon()` 或 `new Icon(path)` **读不了 PNG 压缩帧的 ICO**
—— 而 `tools\make_ico.ps1` 生成的正是这种（每个尺寸都以 PNG 存帧）。
它会把**完全正确**的图标解成噪点，非常容易误判成"图标坏了"。用 Win32 API：

```powershell
# exe 文件图标 —— 资源管理器实际调用的就是这个接口
Add-Type -Namespace W -Name I -MemberDefinition @"
[DllImport("shell32.dll", CharSet=CharSet.Unicode)]
public static extern uint ExtractIconExW(string file, int index, IntPtr[] large, IntPtr[] small, uint count);
"@
$lg = New-Object IntPtr[] 1; $sm = New-Object IntPtr[] 1
$n = [W.I]::ExtractIconExW((Resolve-Path .\dist\sendmecongo-send.exe), 0, $lg, $sm, 1)
"icon groups = $n   large = 0x$($lg[0].ToString('X'))   small = 0x$($sm[0].ToString('X'))"
# 期望：n >= 1，且两个句柄都非 0
```

窗口图标用 `WM_GETICON` 从真实窗口句柄读回：`ICON_SMALL` / `ICON_BIG` 非 0，
并且**窗口类图标为 0** —— 类图标为 0 才说明这个图标是程序显式设上去的，而不是系统默认值兜的。

---

## 4. 验收清单

| 项目 | 命令 / 判据 |
|---|---|
| 测试全绿 | `cargo test --workspace` → 当前应为 **109 passed / 0 failed** |
| 构建零警告 | `cargo build --release` 不输出任何 `warning:` |
| 版本信息 | exe「属性 → 详细信息」里产品名与版本号非空（由 `winres` 的资源段提供） |
| 文件图标 | `ExtractIconExW` 返回 `>= 1`，句柄非 0（§3.3） |
| 窗口图标 | `WM_GETICON` 的 SMALL / BIG 非 0，classIcon = 0 |
| 无运行时依赖 | MSVC：`dumpbin /dependents dist\sendmecongo-send.exe`；GNU：`objdump -p dist\sendmecongo-send.exe \| findstr "DLL Name"`。只应看到系统 DLL，**不应出现** `libgcc_s_seh-1.dll` 或 `libwinpthread-1.dll` |
| 命令行可用 | `dist\sendmecongo-recv.exe --help` 正常输出；`dist\sendmecongo-send.exe --help` 同理 |
| GUI 能开 | 双击两个 exe，各存活 ≥ 8 秒不崩 |

---

## 5. 重新生成图标（只有改了图标几何才需要）

`crates\*\app.ico` 是随仓库提供的成品，**普通构建不需要碰它**。确实要重新生成时：

```cmd
cargo run --release -p sendmecongo-bench -- icon --variant send --out target\send.png --size 256
cargo run --release -p sendmecongo-bench -- icon --variant recv --out target\recv.png --size 256
powershell -ExecutionPolicy Bypass -File tools\make_ico.ps1
```

> ⚠️ `tools\build-windows.ps1` 每次构建都会跑 `make_ico.ps1`，因此它要求
> `target\send.png` 与 `target\recv.png` 已经存在；直接跑它会因为找不到这两个 PNG 而失败。
> `tools\build-windows.cmd` 不会重跑图标生成 —— 这也是推荐用 cmd 那条路的原因之一。

---

## 6. 把产物发出去

- 文件名**不带版本号**：`sendmecongo-send.exe`、`sendmecongo-recv.exe`。这是 release 资产表的固定命名。
- 上传到对应的 release 页面；同名资产会被替换。
- 上传前算校验和，并把它填进 release 说明的资产表：

  ```cmd
  certutil -hashfile dist\sendmecongo-send.exe SHA256
  ```

- 若这次 exe 对应的 release 与源码版本号不一致（exe 内嵌的版本号在编译时就定死了），
  在说明里写清它挂在哪个版本下、为什么。

---

## 7. 常见报错对照

| 现象 | 原因 / 处理 |
|---|---|
| `link.exe not found` | MSVC 缺 VS Build Tools；或改走 GNU：`rustup default stable-x86_64-pc-windows-gnu` |
| 编译期报 avx512 / target feature 相关错误 | rustc 低于 1.89，见 §1 |
| 首次构建 `sendmecongo-recv` 很慢 | 正常：要编译 openh264 的 C++ 源码 |
| `cargo:warning=...本次产物不会带图标` | 交叉编译且宿主缺 `windres` / `rc.exe`。**在 Windows 上原生构建不会出现这条**，出现就说明不是原生构建 |
| 控制台中文乱码 | `chcp 65001`（`build-windows.cmd` 已自带） |
| 离线环境构建失败 | `rust-toolchain.toml` 固定了 1.98.1，离线时拉不到工具链；需预装 |
| exe 图标在别的机器上变白 | Windows 图标缓存，不是构建问题 |
