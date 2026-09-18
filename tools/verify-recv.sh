#!/bin/sh
# sendmecongo-recv 验收矩阵：把 recordings/ 里每一段录像跑一遍，与已知的原文件逐字节比对。
#
#   sh tools/verify-recv.sh            # 全部
#   sh tools/verify-recv.sh 07 08      # 只跑指定的几段
#
# 这几段覆盖了单通道/双通道、turbo30/turbo60/megabit、4K30/4K60，
# 是现成的验收矩阵，改完接收端就该重跑一遍。
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN="$ROOT/target/release/sendmecongo-recv"
OUT="$ROOT/out/verify-recv"

if [ ! -x "$BIN" ]; then
    echo "找不到 $BIN —— 先跑：cargo build --release -p sendmecongo-recv"
    exit 1
fi

rm -rf "$OUT"
mkdir -p "$OUT"

# 录像 → 原文件。前面几段是 256KB，06~08 是 2MB。
all_pairs="02:random.bin 03:random.bin 04:random.bin 05:random.bin 06:random2m.bin 07:random2m.bin 08:random2m.bin"
pairs=$all_pairs
if [ $# -gt 0 ]; then
    pairs=""
    for pair in $all_pairs; do
        for want in "$@"; do
            if [ "${pair%%:*}" = "$want" ]; then
                pairs="$pairs $pair"
            fi
        done
    done
fi

printf '%-10s %-14s %-8s %-9s %s\n' 录像 原文件 结果 耗时 统计
printf '%s\n' "-----------------------------------------------------------------------------"

pass=0
fail=0
for pair in $pairs; do
    tag=${pair%%:*}
    orig=${pair##*:}
    video=$(ls "$ROOT/recordings/$tag".* 2>/dev/null | head -1)
    if [ -z "$video" ]; then
        printf '%-10s %s\n' "$tag" "跳过：recordings/ 里找不到"
        continue
    fi

    start=$(date +%s)
    "$BIN" "$video" --out "$OUT/$tag" --compare "$ROOT/testdata/$orig" \
        --json "$OUT/$tag.json" >"$OUT/$tag.log" 2>&1
    status=$?
    wall=$(( $(date +%s) - start ))

    if [ "$status" -eq 0 ]; then
        pass=$((pass + 1))
        verdict="IDENTICAL ✓"
    else
        fail=$((fail + 1))
        verdict="FAILED ✗"
    fi
    # 报告的第一行标签按界面语言变化：中文（简/繁）是「解码 / 解碼」，英文是 "Decode"。
    stats=$(grep -E '^(解码|解碼|Decode)' "$OUT/$tag.log" | head -1 | cut -c1-60)
    [ -z "$stats" ] && stats=$(head -1 "$OUT/$tag.log" | cut -c1-60)
    printf '%-10s %-14s %-8s %-9s %s\n' "$tag" "$orig" "$verdict" "${wall}s" "$stats"
done

echo
echo "通过 $pass 段，失败 $fail 段。日志在 $OUT/"
if [ "$fail" -gt 0 ]; then
    echo "注意：06 段长期收不齐是已知的 —— 那段 4K30 录像当年用 analyze.py 也只有 1229 个"
    echo "      不同符号，而 RaptorQ 需要 K'=1231。素材本身差两个符号，不是工具的问题。"
fi
