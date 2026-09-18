#!/bin/sh
# 一键跑完光学测试的后半程：解码录像 → 导出符号 → RaptorQ 还原 → 和原件比对。
#
#   tools/optical.sh <录像/PNG目录> <原文件> [符号率sym/s]
#
#   tools/optical.sh recordings/01.mov testdata/small.bin 30
#
# 符号率档位: robust→10  balanced→15  turbo15→15  turbo30→30  turbo60→60  megabit→60
set -e

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# analyze.py 需要 opencv + zxing-cpp；解释器可用 AIRGATE_PYTHON 覆盖，默认 python3。
PY=${SENDMECONGO_PYTHON:-python3}

SRC=${1:?用法: tools/optical.sh <录像或PNG目录> <原文件> [符号率]}
ORIG=${2:?用法: tools/optical.sh <录像或PNG目录> <原文件> [符号率]}
RATE=${3:-30}

NAME=$(basename "$SRC")
NAME=${NAME%.*}

echo "── 1/2 解码 $SRC (sym-rate $RATE)"
"$PY" "$ROOT/tools/analyze.py" "$SRC" --sym-rate "$RATE" \
    --dump "$ROOT/out/$NAME.bin" --json "$ROOT/out/$NAME.json"

echo
echo "── 2/2 还原并与 $ORIG 比对"
"$ROOT/target/release/sendmecongo-bench" decode \
    --in "$ROOT/out/$NAME.bin" --out "$ROOT/out/recv" --compare "$ORIG"
