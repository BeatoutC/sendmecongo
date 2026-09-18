#!/bin/sh
# 构建 macOS 产物：
#
#   dist/sendmecongo-send.app   发送端 GUI（隔离内用）
#   dist/sendmecongo-recv.app   接收端（交给隔离外的人，可双击 / 可拖录像上去）
#
#   ./tools/build-macos.sh
#
# sendmecongo-send 是单 exe 双模式，GUI 点「开始播放」时会 spawn 自己，用的路径是
# current_exe()，在 bundle 里即 Contents/MacOS/sendmecongo-send，无需额外处理。
#
# sendmecongo-recv 之所以也要是个 .app：裸二进制在 Finder 里双击只会开一个终端窗口，
# 而接收方是"另一个人"，双击、拖文件才是他会做的事。.app 里那点胶水在
# crates/sendmecongo-recv/src/finder.rs。
set -e

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

VERSION=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')

echo "== 构建 release ($VERSION)"
cargo build --release -p sendmecongo-send
cargo build --release -p sendmecongo-recv
cargo build --release -p sendmecongo-bench

mkdir -p dist

# --- 图标 ---------------------------------------------------------------
# 两端各一套：同一个几何体，接收端是发送端沿竖直轴的镜像，强调色由青换成薄荷。
# 用代码画而不是放一张手绘图，构建就是自足的，两个图标也永远出自同一份几何。
#
# 注意：.icns 在 dist/ 里叫 AppIcon-send / AppIcon-recv，但装进 bundle 时必须改回
# AppIcon.icns —— Info.plist 里的 CFBundleIconFile 写的是 AppIcon，两个 bundle 各自
# 独立，同名不会冲突。
ICON_VARIANTS=""
if command -v iconutil >/dev/null 2>&1 && command -v sips >/dev/null 2>&1; then
    echo "== 生成图标"
    for variant in send recv; do
        ICONSET="$ROOT/dist/AppIcon-$variant.iconset"
        rm -rf "$ICONSET"
        mkdir -p "$ICONSET"
        target/release/sendmecongo-bench icon --variant "$variant" \
            --out "$ICONSET/source.png" --size 1024 >/dev/null
        for spec in "16 icon_16x16" "32 icon_16x16@2x" "32 icon_32x32" "64 icon_32x32@2x" \
                    "128 icon_128x128" "256 icon_128x128@2x" "256 icon_256x256" "512 icon_256x256@2x" \
                    "512 icon_512x512" "1024 icon_512x512@2x"; do
            set -- $spec
            sips -z "$1" "$1" "$ICONSET/source.png" --out "$ICONSET/$2.png" >/dev/null 2>&1
        done
        # iconutil 不认 source.png，它只认上面那 10 个文件名
        mv "$ICONSET/source.png" "$ROOT/dist/AppIcon-$variant.png"
        iconutil -c icns "$ICONSET" -o "$ROOT/dist/AppIcon-$variant.icns" >/dev/null 2>&1
        rm -rf "$ICONSET"
        [ -f "$ROOT/dist/AppIcon-$variant.icns" ] && ICON_VARIANTS="$ICON_VARIANTS $variant"
    done
else
    echo "== 跳过图标（这台机器没有 sips/iconutil）"
fi

ICON_LINE=""
[ -n "$ICON_VARIANTS" ] && ICON_LINE="  <key>CFBundleIconFile</key><string>AppIcon</string>"

# --- bundle 显示名本地化 -------------------------------------------------
# Dock、菜单栏和访达里显示的是 CFBundleDisplayName，系统按**用户的语言偏好**去
# Contents/Resources/<语言>.lproj/InfoPlist.strings 里取，和 app 内部的语言选择器是
# 两条独立的路 —— 在窗口里选 English 不该改掉 Dock 里的名字，反过来系统语言是英文
# 的人也不该看到「SendMeCongo 发送」。Info.plist 里那份留着兜底（找不到匹配的 lproj 时用）。
#
# 传参：<bundle> <zh-Hans 名> <zh-Hant 名> <en 名> <zh-Hans 文档类型名> <zh-Hant> <en>
# 文档类型名可以为空（发送端没有 CFBundleDocumentTypes）。
localize_bundle() {
    bundle="$1"
    for spec in "zh-Hans|$2|$5" "zh-Hant|$3|$6" "en|$4|$7"; do
        lang=${spec%%|*}
        rest=${spec#*|}
        app_name=${rest%%|*}
        type_name=${rest#*|}
        mkdir -p "$bundle/Contents/Resources/$lang.lproj"
        {
            printf '"CFBundleDisplayName" = "%s";\n' "$app_name"
            if [ -n "$type_name" ]; then
                printf '"CFBundleTypeName" = "%s";\n' "$type_name"
            fi
        } > "$bundle/Contents/Resources/$lang.lproj/InfoPlist.strings"
    done
}

# --- 发送端 -------------------------------------------------------------
APP="$ROOT/dist/sendmecongo-send.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/sendmecongo-send "$APP/Contents/MacOS/sendmecongo-send"
[ -f "$ROOT/dist/AppIcon-send.icns" ] && \
    cp "$ROOT/dist/AppIcon-send.icns" "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>sendmecongo-send</string>
  <key>CFBundleDisplayName</key><string>SendMeCongo 发送</string>
  <key>CFBundleIdentifier</key><string>local.sendmecongo.send</string>
  <key>CFBundleDevelopmentRegion</key><string>zh-Hans</string>
  <key>CFBundleLocalizations</key>
  <array><string>zh-Hans</string><string>zh-Hant</string><string>en</string></array>
  <key>CFBundleExecutable</key><string>sendmecongo-send</string>
  <key>CFBundlePackageType</key><string>APPL</string>
$ICON_LINE
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Internal use only</string>
</dict>
</plist>
PLIST

localize_bundle "$APP" "SendMeCongo 发送" "SendMeCongo 傳送" "SendMeCongo Sender" "" "" ""

# --- 接收端 -------------------------------------------------------------
# 独立的一个 app：整体拷给隔离外的人，他不用装任何东西。
RECV_APP="$ROOT/dist/sendmecongo-recv.app"
rm -rf "$RECV_APP"
mkdir -p "$RECV_APP/Contents/MacOS" "$RECV_APP/Contents/Resources"
cp target/release/sendmecongo-recv "$RECV_APP/Contents/MacOS/sendmecongo-recv"
[ -f "$ROOT/dist/AppIcon-recv.icns" ] && \
    cp "$ROOT/dist/AppIcon-recv.icns" "$RECV_APP/Contents/Resources/AppIcon.icns"

# CFBundleDocumentTypes 不是装饰：Finder 只把"它声称能打开的类型"拖给 app，
# 不声明的话把 .mov 拖到图标上会被直接拒绝。LSHandlerRank 用 Alternate，
# 免得把这个 app 变成系统里视频文件的默认打开方式。
cat > "$RECV_APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>sendmecongo-recv</string>
  <key>CFBundleDisplayName</key><string>SendMeCongo 接收</string>
  <key>CFBundleIdentifier</key><string>local.sendmecongo.recv</string>
  <key>CFBundleDevelopmentRegion</key><string>zh-Hans</string>
  <key>CFBundleLocalizations</key>
  <array><string>zh-Hans</string><string>zh-Hant</string><string>en</string></array>
  <key>CFBundleExecutable</key><string>sendmecongo-recv</string>
  <key>CFBundlePackageType</key><string>APPL</string>
$ICON_LINE
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>Internal use only</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>手机录像</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>public.movie</string>
        <string>public.mpeg-4</string>
        <string>com.apple.quicktime-movie</string>
        <string>public.audiovisual-content</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
PLIST

localize_bundle "$RECV_APP" "SendMeCongo 接收" "SendMeCongo 接收" "SendMeCongo Receiver" \
    "手机录像" "手機錄影" "Phone recording"

# ad-hoc 签名：本机与同团队机器可直接运行，避免 Gatekeeper 直接拒绝。
# 先签可执行文件再签 bundle，否则改过内容会让外层签名失效。
for bundle in "$APP" "$RECV_APP"; do
    codesign --force --sign - "$bundle/Contents/MacOS/"* >/dev/null 2>&1 || true
    codesign --force --sign - "$bundle" >/dev/null 2>&1 || \
        echo "   (ad-hoc 签名失败，不影响本机运行)"
done

SIZE=$(du -sh "$APP" | cut -f1)
RECV_SIZE=$(du -sh "$RECV_APP" | cut -f1)
echo ""
echo "== 完成"
echo "   发送端（隔离内）：$APP  ($SIZE)"
echo "           双击启动，或：open $APP"
echo "           命令行播放：$APP/Contents/MacOS/sendmecongo-send --play --in <文件> --preset turbo60 --size 1600"
echo "   接收端（交给隔离外的人）：$RECV_APP  ($RECV_SIZE)"
echo "           整个 app 拷过去。双击开窗口，把录像拖进窗口即可。"
echo "           屏幕顶部菜单栏里的「界面语言 / 介面語言 / Language」可切换界面语言，命令行用 --lang。"
echo "           注意：macOS 把文件拖到 app 的图标上并不会把路径交给 app（系统发的是苹果事件），"
echo "           拖到窗口里可以。"
echo "           命令行仍可用："
echo "           $RECV_APP/Contents/MacOS/sendmecongo-recv 录像.mov --compare 原文件"
