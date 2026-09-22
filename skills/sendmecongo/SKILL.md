---
name: sendmecongo
description: "通过屏幕二维码与手机录像，实现物理隔离机（Air-Gapped）之间的单向高速光学文件传输与断点续传。"
---

# SendMeCongo 技能操作手册

当用户需要从物理隔离环境跨网闸提取文件、或者提供了 SendMeCongo 仓库并期望传输文件时，使用本技能。

本技能引用 [docs/AGENT.md](file:///Users/wuzhongkang/Desktop/sendmecongo/docs/AGENT.md) 中定义的 CLI 与 JSON 契约。

---

## 1. 运行准备（构建）

> [!CAUTION]
> **必须使用 Release 模式构建！**
> Debug 模式下的二维码生成吞吐量会下降 90% 以上，导致播放严重掉帧并引发接收端大量丢帧。

在开始任何传输之前，确保已完成构建：

```bash
cargo build --release -p sendmecongo-send -p sendmecongo-recv
```

主要可执行文件位于：
- 发送端：`target/release/sendmecongo-send`
- 接收端：`target/release/sendmecongo-recv`

---

## 2. 发送流程（隔离机端）

### 步骤 1：文件预检与预估
询问用户待传输的文件路径与偏好档位（默认推荐 `turbo60`）。
使用 `--prepare-only --json` 进行干跑预检：

```bash
target/release/sendmecongo-send --prepare-only --file "<文件路径>" --preset turbo60 --json out/prep.json
```

读取 `out/prep.json`，向用户汇报：
- **文件体积**：原始大小 `raw_bytes` 与压缩后 `container_bytes`（以及采用的压缩算法 `compression`）。
- **源符号数**：`source_symbols`（单符号 `symbol_size` 字节）。
- **预估拍摄时间**：读取 `estimated_seconds.turbo60`，提示用户：“预计完整录制需要约 X 秒”。

### 步骤 2：转达拍摄要领（务必告知用户）
在启动播放前，将以下拍摄口诀清晰转告用户，避免无效重拍：
1. **手机固定**：靠在杯子或支架上，**绝对不要手持**，防运动模糊。
2. **对焦曝光锁定**：对准屏幕长按取景框锁定 **AE/AF**，防止码切换时拉风箱失焦。
3. **拍摄参数**：推荐 **4K 60fps** 录制（双通道档消除拍频的最优设置），格式选「兼容性最佳」（H.264）。
4. **画面占比**：二维码占画面高度 **60%~70%**，避开强烈反光。

### 步骤 3：启动播放
确认用户手机已准备就绪后，启动全屏播放器：

```bash
target/release/sendmecongo-send --file "<文件路径>" --preset turbo60 --play
```

用户按 `ESC` 键即可退出播放。

---

## 3. 接收与续传流程（自由机端）

### 步骤 1：解析录像
当用户将手机拍摄的录像（.mov / .mp4）拷贝至自由机后，运行接收器：

```bash
target/release/sendmecongo-recv "<录像路径>" --out out/recv --json out/result.json
```

检查进程退出状态码并读取 `out/result.json`：

#### 情况 A：退出码 0 (`status: "done"`)
- 文件已成功还原！
- 四层校验（QR 校验 $\to$ 帧 CRC-16 $\to$ 符号 RaptorQ $\to$ 容器 CRC-32）全部通过。
- 交付文件路径：`received_file`。向用户汇报还原成功与文件大小。

#### 情况 B：退出码 2 (`status: "partial"`)
- 说明录像时长不足，已收到部分符号并自动在录像同级落盘了检查点：
  - 清单索引：`<录像路径同名>.smr.json`
  - 符号载荷：`<录像路径同名>.smr.bin`
- 提取 `out/result.json` 中的字段：
  - `resume_code`：形如 `SMR1-XXXX-XXXX-...` 的补播码。
  - `progress.needed_estimate`：还差约多少个符号。
  - `progress.eta_seconds.turbo60`：按 turbo60 预估还需补拍多少秒。
- **引导用户续传**（二选一）：
  - **方案 1（省时推荐：补播码）**：将 `resume_code` 呈现给用户，让用户在隔离机发送端执行：
    ```bash
    target/release/sendmecongo-send --file "<原文件>" --resume-code "<SMR1-...>" --play
    ```
    发送端将只播放缺额符号（通常只需几秒到十秒），录制后传回。
  - **方案 2（常规兜底：继续循环）**：发送端维持原播放，用户再录制一段视频。
- **合并检查点**：
  拿到补拍的新录像后，带上上次的清单路径执行合并续传：
  ```bash
  target/release/sendmecongo-recv "<新录像路径>" --resume "<上次录像同名>.smr.json" --out out/recv --json out/result_2.json
  ```
  以此类推，直到返回退出码 0。

#### 情况 C：退出码 1
- 发生致命错误（如录像无法打开、全程未识别到二维码）。参考下方排障速查表引导排查。

---

## 4. 翻车对照与排障速查表

| 现象 | 根因 | 处理方式 |
|---|---|---|
| `cannot open video` | 录像为 HEVC 格式且环境缺少解码器 | iPhone 相机设置改「兼容性最佳」，或转码 `ffmpeg -i in.mov -c:v libx264 out.mp4` |
| 一个符号都没解出 | 窗口被遮挡 / 亮度太低 / 对焦没锁 | 确认播放窗口置顶，提高屏幕亮度，长按锁定 AE/AF |
| 仅前几秒解出，后面全是黑洞 | 相机自动对焦拉风箱 | 必须长按取景框锁定 AE/AF 后再开录 |
| `codes/frame` 在 1.0↔2.0 摆动 | 双通道相位拍频（30fps 相机 vs 30fps 双通道） | 切换为 **4K60** 拍摄（相机帧率必须为通道刷新的 2 倍） |
| `codes/frame` 稳定在 1.0 | 某一路通道失焦或偏出取景框 | 调整手机角度与距离，确保左右两个码均清晰可见 |
| `INCOMPLETE` 但符号数已接近 K | $K' > K$（RaptorQ 极小冗余开销） | 正常现象，让流继续循环多录 3~5 秒或使用补播码 |

---

## 5. 红线与边界

1. **单文件体积限制**：协议与内存保护上限为 **64 MB**。大于 64 MB 的文件需先在用户侧拆分卷。
2. **不擅自覆盖**：接收还原文件时，若输出目标目录已有同名文件，应明确提示用户。
3. **严格遵守三元组兼容性**：续传合并与补播码校验依赖 `(session, object_len, symbol_size)`。严禁在续传过程中更换档位或修改原文件。

