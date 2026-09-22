#!/bin/sh
# sendmecongo-recv 验收矩阵。
#
#   sh tools/verify-recv.sh            # 比对档：全部配对逐字节比对
#   sh tools/verify-recv.sh 07 08      # 只跑指定的几段
#   sh tools/verify-recv.sh --smoke    # 冒烟档：不需要基准文件，每段录像都跑
#
# 比对档覆盖单通道/双通道、turbo30/turbo60/megabit、4K30/4K60，每段都得有
# testdata/ 里的原件才跑得起来 —— **没有配对原件的录像它天生覆盖不到**，
# 而系统相机录的安卓 mp4 恰好就是这一类（原件在隔离机里，拿不出来比对）。
# 冒烟档补的就是这个缺口：只问「打开没有、跑通没有」，不问「还原得对不对」。
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN="$ROOT/target/release/sendmecongo-recv"
OUT="$ROOT/out/verify-recv"
SMOKE_OUT="$ROOT/out/verify-recv-smoke"

# 冒烟档：不需要基准文件，recordings/ 里**每一段**录像都跑一遍。
#
# 只回答两个问题：录像打开得了吗，流程跑得完吗。
# 判据是退出码 —— 0（完整还原）和 2（部分接收）都算通路正常；部分接收是合法的
# 中间状态，录像录得不够长本来就是这样。只有 1 算翻车，那说明录像根本没被打开，
# 或者解码链路断了。
smoke() {
    rm -rf "$SMOKE_OUT"
    mkdir -p "$SMOKE_OUT/link"

    printf '%-14s %-7s %-9s %s\n' 录像 退出码 结论 摘要
    printf '%s\n' "-------------------------------------------------------------------------"

    found=0
    pass=0
    fail=0
    for video in "$ROOT"/recordings/*; do
        case "$video" in
            *.mov|*.MOV|*.mp4|*.MP4|*.m4v) ;;
            *) continue ;;
        esac
        found=$((found + 1))
        name=$(basename "$video")
        tag=${name%.*}

        # recv 把续传清单写在录像旁边（<名字>.smr.json）。直接在 recordings/ 里跑
        # 会往那儿丢进度文件，所以从临时目录软链接过去 —— 链接旁边就是临时目录。
        ln -sf "$video" "$SMOKE_OUT/link/$name"

        start=$(date +%s)
        "$BIN" "$SMOKE_OUT/link/$name" --out "$SMOKE_OUT/out/$tag" \
            >"$SMOKE_OUT/$tag.log" 2>&1
        status=$?
        wall=$(( $(date +%s) - start ))

        case "$status" in
            0) pass=$((pass + 1)); verdict="完整 ✓" ;;
            2) pass=$((pass + 1)); verdict="部分 ✓" ;;
            *) fail=$((fail + 1)); verdict="翻车 ✗" ;;
        esac

        # 打开阶段的失败最值得先看 —— 打不开的话，后面的话都没意义。
        why=$(grep -m1 '打开录像失败' "$SMOKE_OUT/$tag.log")
        if [ -z "$why" ] && [ "$status" -eq 0 ]; then
            # 还原行里那条临时目录的完整路径只是噪音，只留文件名。
            why=$(grep -m1 '^还原' "$SMOKE_OUT/$tag.log" |
                awk '{ n = $2; sub(/.*\//, "", n); print $1, n, $3, $4 }')
        fi
        if [ -z "$why" ]; then
            # 没收全的时候，"还原"那行说的是进度存哪了，不是收了账 —— 看符号数。
            why=$(grep -m1 -E '^(符号|解码)' "$SMOKE_OUT/$tag.log")
        fi
        [ -z "$why" ] && why=$(head -1 "$SMOKE_OUT/$tag.log")
        printf '%-14s %-7s %-9s %s\n' "$tag" "$status" "$verdict" \
            "$(echo "$why" | tr -s ' ' | cut -c1-58)"
    done

    echo
    if [ "$found" -eq 0 ]; then
        echo "recordings/ 里一段录像都没有，没什么可冒烟的。"
        return 1
    fi
    echo "冒烟通过 $pass 段，翻车 $fail 段（共 $found 段）。日志在 $SMOKE_OUT/"
    if [ "$fail" -gt 0 ]; then
        echo "翻车 = 录像根本没打开。先看 $SMOKE_OUT/<录像>.log 里的「打开录像失败」那一行。"
        return 1
    fi
    return 0
}

if [ ! -x "$BIN" ]; then
    echo "找不到 $BIN —— 先跑：cargo build --release -p sendmecongo-recv"
    exit 1
fi

if [ "${1:-}" = "--smoke" ]; then
    smoke
    exit $?
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

    # recv 把续传清单写在录像旁边（<名字>.smr.json）。直接喂 recordings/ 里的
    # 原路径会往那儿丢一堆进度文件，所以从输出目录软链接过去 —— 链接旁边就是
    # out/，那些文件随每次重跑一起被清掉，recordings/ 始终是干净的。
    mkdir -p "$OUT/link"
    name=$(basename "$video")
    ln -sf "$video" "$OUT/link/$name"
    video="$OUT/link/$name"

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
