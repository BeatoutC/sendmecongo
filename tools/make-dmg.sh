#!/bin/sh
# 把 .app 打成可以直接发人的 DMG。
#
#   ./tools/make-dmg.sh            # 接收端（默认，发给隔离外的人）
#   ./tools/make-dmg.sh send       # 发送端
#   ./tools/make-dmg.sh both       # 两个都打
#
# 为什么不是直接发 .app：
#   1. .app 是个目录。过邮件/IM/网盘很容易被拆散或丢掉可执行位，收到的人看到的是
#      一堆文件而不是一个应用。
#   2. 单个 .dmg 传过去、双击挂载、把 app 拖进「应用程序」，是 mac 用户唯一不用解释的流程。
#   3. 挂载点里可以放"第一次打开"的说明，和被系统拦下来时的一键补救。
#
# 只做 arm64（Apple Silicon）。Intel mac 跑不了，这是有意的：本机就是 arm，
# 交叉编 x86_64 要连 openh264 的 C++ 一起编，收益不值这个复杂度。
set -e

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

WHICH=${1:-recv}
case "$WHICH" in
    recv|send|both) ;;
    *) echo "用法：$0 [recv|send|both]"; exit 1 ;;
esac

VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')

# 刚跑过 tools/build-macos.sh 的时候，dist 里的 .app 就是最新的，再构建一遍
# 只是把已经建好的 bundle 和图标中间产物删掉重建 —— 慢，而且没必要。
# SMC_SKIP_BUILD=1 直接用现有产物，只重打 DMG。
if [ "${SMC_SKIP_BUILD:-0}" = "1" ]; then
    echo "== 跳过构建（SMC_SKIP_BUILD=1，用 dist 里现有的 app）"
else
    echo "== 构建两个 app（含图标）"
    ./tools/build-macos.sh >/dev/null
fi

# --- 卷里放什么 ---------------------------------------------------------
# 说明文件是这里最要紧的东西：app 是 ad-hoc 签名的，被下载过一次之后
# macOS 会加上隔离标记，第一次打开必然要人手动放行。写清楚比事后解释便宜。
#
# 安装与"第一次打开"两段两个 app 一模一样，所以合在一起写；「怎么用」必须
# 分开 —— 发送端和接收端是同一条链路上完全不同的两份工作，一份通用说明
# 只会把两边都说糊涂（发送端的卷里写着"把 sendmecongo-recv 拖进应用程序"，
# 是之前真发生过的错误）。

# $1 目标文件  $2 app 名（不带 .app）
write_readme_common() {
    cat > "$1" <<TXT
第一次打开会被 macOS 拦一下，这是正常的
────────────────────────────────────────

为什么：这个 app 没有苹果开发者签名（那是要花钱的开发者账号，
内部工具不值当），所以从网上/邮件/AirDrop 传过来之后，
macOS 会给它打一个"来自网络"的标记，第一次打开会拒绝。

按下面做一次，以后就再也不会问了。


一、安装

   把 $2.app 拖到旁边的「Applications」（中文系统里显示成「应用程序」）上，
   或者直接拖进「应用程序」文件夹。


二、第一次打开（只需一次）

   情况 A：右键（或按住 Control 点）app 图标 → 选「打开」→ 再点一次「打开」。

   情况 B：如果双击之后只弹出"无法验证开发者"，而且没有「打开」按钮
           （macOS 15 之后取消了那个按钮）：
           打开「系统设置」→「隐私与安全性」→ 往下拉到「安全性」，
           会看到一行说 $2.app 被阻止了，点右边的「仍要打开」。

   从那以后，正常双击就能用。


三、如果还是不行

   本卷里有一个「清除下载隔离标记.command」。
   把 app 放进「应用程序」之后，双击它，它会把这个 app 上的"来自网络"
   标记去掉。脚本内容可以右键 →「用文本编辑打开」自己看，只有一行 xattr。

TXT
}

# $1 目标文件  $2 app 名  $3 app 里的可执行文件名
write_readme_recv() {
    cat >> "$1" <<TXT

四、怎么用

   双击 $2.app，出现窗口。最快的用法是：把 app 和手机录像放进同一个文件夹，
   再双击 app —— 它会把同目录（以及下载 / 桌面 / 影片 / 外接盘）里的录像
   列出来，点一个就开始还原。

   窗口里没有别的问题要回答：进度、已解码帧数、已收符号数都在上面，随时可以停；
   完成后能直接在访达里定位还原出来的文件。

   想再传给别人：整个 $2.app 拷走就行，对方的 mac 上不需要装任何东西
   （不需要 Python、不需要解码器、不需要联网）。


五、给习惯命令行的人

   /Applications/$2.app/Contents/MacOS/$3 录像.mov

   加 --compare 原文件 可以顺便逐字节比对，加 --json 结果.json 可以拿到机器可读的统计。
   不带参数运行会直接开窗口；要在脚本里用命令行，前面加 SENDMECONGO_NO_DIALOG=1。

   界面有三种语言：简体中文 / 繁體中文 / English。点屏幕顶部菜单栏里
   「界面语言 / 介面語言 / Language」切（就在 app 菜单右边），立即生效；
   命令行用 --lang zh-Hant（或 zh-Hans / en），默认跟随系统语言。


$2   版本 $VERSION   ·   仅支持 Apple Silicon（M 系列）Mac
TXT
}

# $1 目标文件  $2 app 名  $3 app 里的可执行文件名
write_readme_send() {
    cat >> "$1" <<TXT

四、怎么用

   双击 $2.app，出现配置窗口。

   1. 选文件 —— 点「选择文件…」，或者直接把文件拖进窗口。
      它搬的是文档、密钥、配置文件、小体积证据包；不是大文件传输工具。
   2. 选档位 —— 默认 turbo60 是最稳的一档。偏远、屏幕小的场合退到 turbo30。
   3. 按「开始播放」，整个窗口会被二维码流占满。
   4. 另一台机器上的手机对准屏幕全程录像。录多久窗口上写着
      （「建议录像 ≥ X 秒」那一条）—— 宁可录长，别录短。

   几条实测出来的拍摄要求（播放前窗口里也列着，别跳过）：

     · 长按取景框锁定 AE/AF，否则自动对焦会周期性拉风箱
     · 手机固定住（用支架），录制中途移动会让模板失焦
     · 打开勿扰模式，通知横幅会挡住屏幕
     · 档位是双通道时必须用 4K60 拍摄

   播放中按 ESC 停止。接收方需要什么、怎么把文件还原出来，
   配置窗口里的「接收说明」一页已经写全了 —— 那部分可以直接截图发给对方。


五、给习惯命令行的人

   /Applications/$2.app/Contents/MacOS/$3 --play --in <文件> --preset turbo60 --size 1600

   加 --lanes 2 走双通道（此时 --size 要填每码边长的两倍）。

   界面有三种语言：简体中文 / 繁體中文 / English。点屏幕顶部菜单栏里
   「界面语言 / 介面語言 / Language」切（就在 app 菜单右边），立即生效；
   命令行用 --lang zh-Hant（或 zh-Hans / en），默认跟随系统语言。


$2   版本 $VERSION   ·   仅支持 Apple Silicon（M 系列）Mac
TXT
}

# $1 目标文件  $2 app 名
write_quarantine_fixer() {
    cat > "$1" <<'CMD'
#!/bin/sh
# 去掉这个 app 上"来自网络"的标记，让 macOS 不再拦它。
# 这一行只影响这一个 app，不动系统里别的任何东西。
set -e
cd "$(dirname "$0")"
APP="/Applications/__APP__.app"
if [ ! -d "$APP" ]; then
    echo "没在 /Applications 里找到 __APP__.app。"
    echo "请先把本卷里的 __APP__.app 拖进「应用程序」，再运行本脚本。"
    printf "按回车关闭…"; read -r _; exit 1
fi
echo "正在清除隔离标记：$APP"
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true
codesign --verify --verbose=1 "$APP" 2>&1 | tail -2 || true
echo
echo "好了。现在双击 __APP__.app 应该就能打开。"
printf "按回车关闭…"; read -r _
CMD
    # 用占位符而不是让 heredoc 展开：正文里有 $(dirname "$0") 和 $APP，
    # 它们必须原样落进文件，交给以后双击它的人去展开。
    sed -i '' "s/__APP__/$2/g" "$1"
}

# $1 app 名（= bundle 名，不带 .app）  $2 app 里的可执行文件名
# $3 dmg 文件名主干  $4 卷标  $5 recv|send
make_dmg() {
    app_name=$1
    exe_name=$2
    dmg_stem=$3
    vol_name=$4
    kind=$5
    APP="$ROOT/dist/$app_name.app"
    DMG="$ROOT/dist/${dmg_stem}-${VERSION}.dmg"
    # 暂存区放系统临时目录，而不是 dist/：dist 只该放要交付的东西，打包中途失败时
    # 也不会在那里留下半个目录（真发生过——第一次打 send 的 DMG 时中断，dist 里就
    # 多出一个 .dmg-sendmecongo-send 需要手工清掉）。
    STAGE="$(mktemp -d "${TMPDIR:-/tmp}/smc-dmg-$dmg_stem.XXXXXX")"

    [ -d "$APP" ] || { echo "   找不到 $APP"; return 1; }

    # $STAGE 由上面的 mktemp 建好了；同名的旧 $DMG 交给 hdiutil 的 -ov 覆盖。
    cp -R "$APP" "$STAGE/"
    # 惯例：一个指向 /Applications 的链接，中文系统里会自动显示成「应用程序」
    ln -s /Applications "$STAGE/Applications"

    README="$STAGE/第一次打开请看这里.txt"
    write_readme_common "$README" "$app_name"
    case "$kind" in
        recv) write_readme_recv "$README" "$app_name" "$exe_name" ;;
        send) write_readme_send "$README" "$app_name" "$exe_name" ;;
    esac
    write_quarantine_fixer "$STAGE/清除下载隔离标记.command" "$app_name"
    chmod +x "$STAGE/清除下载隔离标记.command"

    echo "== 打包 $dmg_stem-$VERSION.dmg"
    hdiutil create -volname "$vol_name" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
    rm -rf "$STAGE"

    hdiutil verify "$DMG" >/dev/null
    size=$(du -h "$DMG" | cut -f1)
    sum=$(shasum -a 256 "$DMG" | cut -d' ' -f1)
    echo "   完成：$DMG  ($size)"
    echo "   sha256 $sum"
}

# 两端的 bundle 名与 dmg 名主干现在一致（sendmecongo-send / sendmecongo-recv），
# 卷内 app 名、说明文里的 app 名、可执行文件名都从同一份参数出来。
case "$WHICH" in
    recv) make_dmg sendmecongo-recv sendmecongo-recv sendmecongo-recv "SendMeCongo 接收" recv ;;
    send) make_dmg sendmecongo-send sendmecongo-send sendmecongo-send "SendMeCongo 发送" send ;;
    both)
        make_dmg sendmecongo-recv sendmecongo-recv sendmecongo-recv "SendMeCongo 接收" recv
        echo
        make_dmg sendmecongo-send sendmecongo-send sendmecongo-send "SendMeCongo 发送" send
        ;;
esac

echo ""
echo "== 发出前自己验一遍（可选，但很值）"
echo "   open dist/sendmecongo-recv-${VERSION}.dmg          # 挂载看看内容对不对"
echo "   挂载点里直接跑一次："
echo "   '/Volumes/SendMeCongo 接收/sendmecongo-recv.app/Contents/MacOS/sendmecongo-recv' --help"
