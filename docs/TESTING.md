# 光学链路测试手册

数字域（编码 → RaptorQ → CRC → 还原）已经全绿，验证过 `IDENTICAL ✓`。
**剩下的唯一未知是光学域**：屏幕 → 空气 → 摄像头这一段到底交付多少。这份手册就是测它的。

要回答三个问题：

1. 摄像头能不能稳定读出我们渲染的码？（解码率）
2. 每个档位在「你的屏幕 + 你的手机」上真实吞吐是多少？（goodput，不是标称值）
3. 最差情况（歪一点、远一点、有反光）还能不能还原？

---

## 0. 五分钟最小闭环

先别追求数据，先把「屏幕 → 手机 → 文件」这条路走通一次。用最小样本，一循环只要 0.8 秒。

```bash
# 仓库根目录下：

# 终端 A：无限循环播放（窗口会置顶，ESC 停止）
./target/release/sendmecongo-bench play --in testdata/small.bin --preset turbo30 --size 1200
```

手机正对屏幕录像 **5 秒** → 传回电脑（放到 `recordings/`）→ 然后：

```bash
# 解码录像 + 导出符号（需要 python3 + opencv + zxing-cpp，见 tools/analyze.py）
python3 tools/analyze.py recordings/01.mov --sym-rate 30 --dump out/01.bin

# 还原文件并和原件比对
./target/release/sendmecongo-bench decode --in out/01.bin --out out/recv \
  --compare testdata/small.bin
```

看到 `compare  IDENTICAL ✓` 就说明整条链路成立。看到 `INCOMPLETE` 也别慌——它只是说符号不够，接着看第 3 节判读。

### 0b. 一条命令的版本：sendmecongo-recv

上面那三步（analyze.py 解码 → dump → bench 还原）是**测量**用的，它需要 Python + OpenCV + zxing-cpp。
接收方不该被要求装这些，所以日常还原走 `sendmecongo-recv`：

```bash
cargo build --release -p sendmecongo-recv
./target/release/sendmecongo-recv recordings/08.MOV --compare testdata/random2m.bin
```

它内部就是上面那条链路的纯 Rust 实现：自研 ISO-BMFF 解封装 → `rusty_h265` / `openh264` 解码
→ `rxing` 识别 → 复用 `sendmecongo-core` 的 `Receiver` 还原。差别只在实现，协议和判读标准完全一致。

**`--dump` 是两条实现之间的交叉复核**：`sendmecongo-recv` 导出的符号文件，
可以直接喂给 `sendmecongo-bench decode`，两者必须都得出 `IDENTICAL ✓`。

```bash
./target/release/sendmecongo-recv recordings/04.MOV --dump /tmp/s.bin --quiet
./target/release/sendmecongo-bench decode --in /tmp/s.bin --out /tmp/recv --compare testdata/random.bin
```

**验收矩阵**（改完接收端就该重跑一遍）：

```bash
sh tools/verify-recv.sh          # 全部 7 段
sh tools/verify-recv.sh 07 08    # 只跑指定几段
```

**接收方那边出问题时怎么定位**：设 `SENDMECONGO_DUMP_FRAME=<第几帧>:<输出.pgm>`，
它会把解码出来的那一帧灰度图原样写出来。ppm/pgm 可以直接用 `cv2.imread` 读，
拿它跟"相机里看到的"对比，就能分清是**解码错**还是**识别不到**。

### 0c. 测 macOS 的 .app

`dist/sendmecongo-recv.app` 是给接收方的形态，改完 `gui.rs` / `finder.rs` 或打包脚本要验四件事：

```bash
plutil -lint dist/sendmecongo-recv.app/Contents/Info.plist   # plist 合法
codesign -v --verbose=2 dist/sendmecongo-recv.app             # 签名有效
cargo test -p sendmecongo-recv                                # 10 个用例，含进度计算与 osacompile
```

**开窗口的冒烟测试**（`ps` 在沙箱里不可用，用 `kill -0`）：

```bash
./target/release/sendmecongo-recv --gui >/tmp/o 2>/tmp/e &
PID=$!; sleep 6; kill -0 $PID && echo "窗口存活"; kill $PID
```

**窗口路径能不能真的还原出文件**——给它一个录像和一个输出目录，等一会儿看产物：

```bash
./target/release/sendmecongo-recv --gui recordings/05.MOV --out /tmp/guirun &
PID=$!; sleep 16; ls /tmp/guirun; kill $PID
```

> 命令行模式不受窗口影响：`--gui` 只是显式开门，不带它就是原来那条 CLI。
> `SENDMECONGO_NO_DIALOG=1` 会把"包内启动"也压回命令行行为，供脚本和上面这类测试使用。

> **拖放到 app 图标上不会传路径**（LaunchServices 发 `odoc` 苹果事件，命令行程序收不到，
> 进程拿到的 `argc=0`，实测确认）。但拖到**窗口里**可以，那是 egui 收的。
> 想测图标拖放那条路，就把 app 和一个录像放进同一个空目录再跑无参启动。

### 0d. 测要发出去的那个 dmg

接收方拿到的是 DMG，不是 dist 里的 app 目录。改完打包脚本一定要**挂起来**验一遍——
只验 dist/ 里的 app 是验不到 dmg 里的说明文件和链接的。

```bash
./tools/make-dmg.sh recv
hdiutil verify dist/sendmecongo-recv-0.4.1.dmg
hdiutil attach dist/sendmecongo-recv-0.4.1.dmg -nobrowse -quiet
ls "/Volumes/SendMeCongo 接收/"        # 应有 app / Applications 链接 / 说明 / .command
# 从只读卷里直接跑一次，这是对方实际会遇到的运行条件
SENDMECONGO_NO_DIALOG=1 "/Volumes/SendMeCongo 接收/sendmecongo-recv.app/Contents/MacOS/sendmecongo-recv" \
    recordings/05.MOV --out /tmp/dmgcheck --compare testdata/random.bin
hdiutil detach "/Volumes/SendMeCongo 接收"
```

`lipo -info` 应显示 `arm64`（非 fat）—— 只支持 Apple Silicon 是有意的，见 README。

---

## 1. 环境准备（这段最容易翻车）

摄像头这条链路，物理条件比参数重要。按下面做，能省掉大半的无效测试。

**手机端**
- 录像格式设为 **H.264「兼容性最佳」**（iPhone：设置 → 相机 → 格式）。默认的 HEVC 高效格式 OpenCV 经常读不了，会直接报 `cannot open video`。
- 分辨率选 **4K/30fps** 优先。二维码是高密度码，**分辨率比帧率重要**——4K30 通常赢过 1080p60。
- 长按取景框锁定 **AE/AF（自动曝光/对焦）** 后再开始录。自动对焦会在码切换时反复拉风箱，那是丢帧的主要来源。
- 手机 **必须固定**。手持抖动 = 运动模糊 = 大量丢帧。靠在杯子上、用支架，怎么都行，别用手端。

**屏幕端**
- 亮度拉满，关掉夜览 / True Tone / 自动亮度调节。
- 关掉屏幕上的其他窗口，避免背景干扰；播放窗口本身是白底，别让别的窗口压在上面（窗口已置顶，但全屏应用会盖住）。
- `--size` 给 1000~1400，保证窗口**完整可见且不被遮挡**。不要手动拖拽拉伸窗口——非正方形模块会让解码率暴跌。窗口是正方形，全屏时 macOS 会保持比例加黑边，是安全的。

**几何**
- 正对屏幕，距离让二维码占据取景框 **60~70%** 高度。贴满边缘会吃到镜头边缘畸变。
- 避开灯光和窗户在屏幕上的反光，斜 15° 以上的反光足够毁掉一整片测试。

---

## 2. 分级测试

| 级别 | 目的 | 输入 | 命令 |
|---|---|---|---|
| **L0** 数字域 | 编解码正确（已过） | — | `simulate --in testdata/small.bin --preset turbo30` |
| **L1** 静态光学 | 渲染的码相机认不认 | PNG 帧显示在屏幕上，手机**拍照** | 见下 |
| **L2** 动态基线 | 理想条件下的真实吞吐 | 手机**录像** 5~10s | 见下 |
| **L3** 档位扫描 | 各档位在你的设备上能用吗 | 每档各录一段 | 见下 |
| **L4** 场景矩阵 | 最差情况 | 距离/角度/亮度组合 | 见下 |

### L1 · 静态光学（先做这个，30 秒出结果）

动态测失败时，先用静态排除「是不是码根本没渲染对」。

```bash
./target/release/sendmecongo-bench encode --in testdata/small.bin --preset turbo30 \
  --out out/l1 --scale 6
# 打开 out/l1/00000.png 全屏显示，手机拍照
python3 tools/analyze.py recordings/l1/ --dump out/l1.bin
```

单张照片能解出来，就说明渲染 + 相机 + 解析全对，动态失败只可能是帧率/运动模糊问题。

### L2 · 动态基线

就是第 0 节那三条命令，但录 **10 秒**，`--sym-rate` 按档位填（见第 3 节表）。

### L3 · 档位扫描

每个档位录一段，同一距离同一角度。**从慢到快**，一旦某档 `symbol_delivery_ratio` 掉到 30% 以下，后面更快的档基本不用测了。

```bash
for p in turbo15 turbo30 turbo60 megabit; do
  # 每个档位单独 play，录 10 秒，存成 recordings/$p.mov
  # sym-rate 填该档的 sym/s：turbo15→15  turbo30→30  turbo60→60  megabit→60
done
```

### L4 · 场景矩阵

选定一个「可用档位」（建议 turbo30），只变一个变量，其余固定：

| 变量 | 取值 |
|---|---|
| 距离 | 15cm / 25cm / 40cm |
| 角度 | 0° / 15° / 30° |
| 屏幕亮度 | 100% / 50% |
| 手持 vs 固定 | 固定 / 手持 |

每组录 10 秒。这张表就是后面写进验收报告的原始素材。

---

## 3. 判读标准

`analyze.py` 输出里，最值得看的是这几个：

| 字段 | 含义 | 判据 |
|---|---|---|
| `time_to_complete_sec` | **多久真正收齐**（不是录了多久） | 这是吞吐的分母 |
| `goodput_bytes_per_sec` | 真实文件吞吐 = 文件 / 收齐耗时 | 对外报数用这个 |
| `symbol_delivery_ratio` | 相机采样能力里解出多少（分母 = 帧率 × 检测到的通道数） | ≥ 90% 好，60~90% 可用，< 60% 该档吃力 |
| `longest_blackout_sec` | 最长的一段「零解码」 | **> 0.5s 说明有东西挡了镜头**，不是协议问题 |
| `completeness` | 收到的 / 还原所需 | ≥ 1.0 才能解出 |

三个容易踩的坑：

1. **别拿 `goodput` 之外的数当吞吐。** 录像时长 ≠ 收齐耗时。流是循环的，往往你还没停录它就已经收完了，多录的部分纯属冗余。
2. **别拿 `decode_rate` 直接下结论。** 它是全片平均，会被一段遮挡严重拉低——要看 `longest_blackout_sec` 判断是不是被干扰了，被干扰的那段应该剔除再评估。
3. **`unique_symbols` 会饱和。** 一循环只有 N 帧，`unique` 最多就是 N，录再久也不会涨。所以它只能用来判断"收没收齐"，不能用来衡量损耗。
4. **`completeness ≥ 1.0` 也可能解不出。** RaptorQ 需要 `K'` 个符号，`K' ≥ K` 由 RFC 6330 参数决定。小文件通常 K'=K，大文件会多几个（2MB 时 K=1227、K'=1231）。看到 `unique` 只比 K 多几个却报 INCOMPLETE，别怀疑有 bug——继续录，流会循环回来补上。

`rejected_frames` 是 CRC 校验失败的载荷，被正确丢弃了。这个数大说明相机读错了但没乱解——机制工作正常，只是光学条件差。

---

## 4. 结果记录

每测一组填一行，最后汇总成验收报告：

| 档位 | 距离 | 角度 | 亮度 | 录像 | delivery | completeness | goodput | 结论 |
|---|---|---|---|---|---|---|---|---|
| turbo30 | 25cm | 0° | 100% | 10s | | | | |

---

## 5. 翻车对照表

| 现象 | 原因 | 处理 |
|---|---|---|
| `cannot open video` | iPhone 录的 HEVC | 相机设置改「兼容性最佳」，或转码 `ffmpeg -i in.mov -c:v libx264 out.mp4` |
| 一个符号都没解出 | 窗口被遮挡 / 亮度太低 / 对焦没锁 | 先做 L1 静态测试 |
| 只解出前几帧 | 自动对焦拉风箱 | 长按锁定 AE/AF |
| `codes/frame` 在 1.0↔2.0 之间摆动 | **双通道相位拍频**：相机帧率 = 每 lane 更新率，相对相位缓慢漂移 | 换 4K60 拍摄（相机帧率 = 每 lane 的 2 倍）；看 `segment_codes_per_frame` 确认 |
| `codes/frame` 稳定在 1.0（某一路全程读不出） | 该 lane 失焦或被遮挡 | 导出录像帧对比两个码的清晰度 |
| megabit 全灭但 turbo30 正常 | V40 模块太密，你的屏幕/相机分辨率撑不住 | megabit 档需要 4K + 支架 + 近距离，是实验档不是默认档 |
| `INCOMPLETE` 但 `unique` 已接近 K | `K' > K`，就差那几个 | 继续录，下一轮循环会补齐 |

---

## 6. 测完之后

`--dump` 出来的符号可以直接喂给 `decode` 反复重试，不用重录：

```bash
./target/release/sendmecongo-bench decode --in out/07.bin --out out/recv --compare testdata/random2m.bin
```

## 7. 实测基线（汇总）

历史八轮实测的完整过程记录在 git 历史里；这里只留结论数字。条件：MacBook Air 屏幕 →
iPhone 录像、锁定 AE/AF、手机固定，全部 `IDENTICAL ✓`、`overhead 1.00x`。

| # | 输入 | 档位 | 拍摄 | goodput | 备注 |
|---|---|---|---|---|---|
| 1 | 32KB | turbo30 | 1080p30 | 45.8 KB/s | 首轮基线，1.8s 黑洞拉低均值 |
| 2 | 256KB | turbo30 | 4K30 | 49.9 KB/s | 达成率 99%，链路跑满 |
| 4 | 256KB | megabit | 4K30 | 105.2 KB/s | V40 可解性确认（61% 达成） |
| 5 | 256KB | turbo60 | 4K30 | 99.8 KB/s | 双通道布局修复后 2.00 codes/frame |
| 7 | 2MB | turbo60 | 4K30 | 74.6 KB/s | 相位拍频，`codes/frame` 1.0↔2.0 摆动 |
| 8 | 2MB | turbo60 | **4K60** | **100.2 KB/s** | **当前最优**，十段全 2.0 |

**推荐默认配置**：`turbo60` · `--size 1600` · **4K60 拍摄** · 锁定 AE/AF · 手机固定。
按 100 KB/s：1MB ≈ 10s，10MB ≈ 1.7min，64MB ≈ 10min。

两个被实测钉死过的教训：

- **吞吐的理论上限别拿标称值算。** 还原只需 K 个源符号，标称「有效」是按播完整轮（含修复
  符号）的保守口径。turbo30 标称有效 41.7 KB/s，实际可达 ~50 KB/s——实测 49.9「超标」
  是口径问题，不是玄学。
- **播放器输出的 `actual sym/s` 低于目标 90% 会告警**——那种情况的瓶颈在渲染不在光学信道。
  实测参考：单通道 1200px → 29.1 sym/s（97%）；双通道每码 800px → 59.7 sym/s（99%）。
