#!/bin/sh
# 生成一份 Windows 精简构建包快照（默认 ../sendmecongowin），用于把源码带到一台
# 未联网的 Windows 机器上构建。那台机器拉不了仓库，所以这是"搬运"的入口。
#
#   ./tools/make-windows-bundle.sh [目标目录] [--force]
#
# 目录默认不存在也没关系，它就是被生成出来的。
#
# 为什么要有这个脚本：这个包是**手挑出来的子集**，规则有一堆（哪些带、哪些不带、
# 行尾怎么转），手工维护出过一次事故——包被删得只剩一个 crate，而没有任何迹象。
# 把规则写进脚本，它就跑得对、也修得回来。
#
# 两条行尾规则（踩过）：
#   .cmd  必须 CRLF 且**无** BOM —— cmd.exe 会把 BOM 当成命令字符
#   .ps1  必须 CRLF 且带 UTF-8 BOM —— PowerShell 5.1 否则按 GBK 解码，中文全乱
set -e

ROOT=$(cd "$(dirname "$0")/.." && pwd)

FORCE=""
DST=""
for arg in "$@"; do
    case "$arg" in
        --force) FORCE="--force" ;;
        *) DST="$arg" ;;
    esac
done
[ -n "$DST" ] || DST=$(dirname "$ROOT")/sendmecongowin

# --- 闸门 ---------------------------------------------------------------
# 下面会 rm -rf 目标目录。目标可能是别人真正在开发的那棵树（尤其可能指到一盘
# 挂载/同步目录），所以只允许删掉"我们自己生成的东西"：
# 认这个标记文件。没有标记又要删，就停下来问人。
MARKER=".sendmecongo-bundle"
if [ -e "$DST" ] && [ -n "$(ls -A "$DST" 2>/dev/null)" ] && [ ! -f "$DST/$MARKER" ]; then
    echo "拒绝执行：$DST 已存在，而且不是这个脚本生成的（没有 $MARKER 标记）。"
    echo ""
    echo "它会做的是 rm -rf 这个目录。如果那里面有你自己的东西，就已经没了。"
    echo "确认要覆盖的话：$0 $DST --force"
    exit 1
fi
if [ -e "$DST" ] && [ ! -f "$DST/$MARKER" ] && [ -z "$FORCE" ]; then
    echo "拒绝执行：$DST 已存在且没有标记。加 --force 表示确认覆盖。"
    exit 1
fi

echo "源  $ROOT"
echo "目标 $DST"
echo ""

rm -rf "$DST"
mkdir -p "$DST/crates" "$DST/tools" "$DST/docs" "$DST/testdata" "$DST/.cargo"

# --- 构建清单 -----------------------------------------------------------
for file in Cargo.toml Cargo.lock rust-toolchain.toml; do
    cp "$ROOT/$file" "$DST/$file"
done
cp "$ROOT/.cargo/config.toml" "$DST/.cargo/config.toml"

for crate in sendmecongo-core sendmecongo-ui sendmecongo-bench sendmecongo-send sendmecongo-recv; do
    mkdir -p "$DST/crates/$crate"
    cp -R "$ROOT/crates/$crate/." "$DST/crates/$crate/"
    rm -rf "$DST/crates/$crate/target"
done

for doc in PROTOCOL.md TESTING.md; do
    cp "$ROOT/docs/$doc" "$DST/docs/$doc"
done

# 只带 GUI 自测用的小样本，random*.bin 是光学验收用的，Windows 侧用不到。
for sample in small.bin small.txt; do
    [ -f "$ROOT/testdata/$sample" ] && cp "$ROOT/testdata/$sample" "$DST/testdata/$sample"
done

# --- 行尾 ---------------------------------------------------------------
to_crlf() {  # $1 源  $2 目标  $3 = bom | nobom
    tr -d '\r' < "$1" > "$DST/.crlf.tmp"
    if [ "$3" = "bom" ]; then
        { printf '\357\273\277'; cat "$DST/.crlf.tmp"; } | sed 's/$/\r/' > "$2"
    else
        sed 's/$/\r/' "$DST/.crlf.tmp" > "$2"
    fi
    rm -f "$DST/.crlf.tmp"
}

to_crlf "$ROOT/tools/build-windows.cmd" "$DST/tools/build-windows.cmd" nobom
to_crlf "$ROOT/tools/build-windows.ps1" "$DST/tools/build-windows.ps1" bom

# --- README：正文里有一堆 macOS 侧才有的路径，得在这里说清楚 ------------
cp "$ROOT/README.md" "$DST/README.md"
cat >> "$DST/README.md" <<'NOTE'

---

## 在 Windows 上构建（本目录是精简构建包）

这个目录是从主仓库挑出来的、只够构建用的副本。它**不含**：

- `recordings/`、`out/`、`dist/` —— 录像、验收产物、macOS 打包结果
- `tools/analyze.py`、`tools/optical.sh`、`tools/verify-recv.sh`、`tools/build-macos.sh`
  —— 光学测量链和 macOS 打包，都在 Mac 上跑
- `testdata/random*.bin` —— 光学验收用的随机素材

所以上面正文里凡是指到这些路径的地方，在 Windows 侧都会找不到文件，那是预期的。

构建：

    tools\build-windows.cmd

产出 `dist\sendmecongo-send.exe`（发送端，双击进 GUI）和
`dist\sendmecongo-recv.exe`（接收端，交给隔离外的人）。

首次构建 rustup 会联网拉 `rust-toolchain.toml` 指定的 1.98.1；离线机器请把那个文件
改成本机已装的 ≥1.89 版本。`sendmecongo-recv` 还要编 openh264 的 vendored C++ 源码，
需要 C++ 编译器（MSVC 或 mingw 均可），首次明显偏慢。它那个 NASM 汇编步骤失败是
**软失败**，没有 nasm 也能编、也能用。
NOTE

# --- 汇报 ---------------------------------------------------------------
cat > "$DST/$MARKER" <<MARK
这个目录由 sendmecongo/tools/make-windows-bundle.sh 生成，内容是主仓库的精简子集。
脚本下次运行时会整目录重建它，所以不要把它当成唯一副本。
MARK

FILES=$(find "$DST" -type f | wc -l | tr -d ' ')
SIZE=$(du -sh "$DST" | cut -f1)
echo "== 完成：$DST  ($FILES 个文件, $SIZE)"
echo ""
find "$DST" -type f | sed "s|$DST/||" | sort | sed 's/^/   /'
echo ""
echo "== 行尾自检"
for file in "$DST/tools/build-windows.cmd" "$DST/tools/build-windows.ps1"; do
    head -c 3 "$file" | od -An -tx1 | tr -d ' \n' | grep -q efbbbf \
        && bom="BOM" || bom="无 BOM"
    crlf=$(head -c 200 "$file" | od -c | grep -c '\\r' || true)
    printf '   %-40s %s, CRLF=%s\n' "$(basename "$file")" "$bom" "$crlf"
done
