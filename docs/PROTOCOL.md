# SMQ1 — SendMeCongo 光学传输协议 v0.1

自研协议。设计目标：单向、无 ACK、可乱序、可中途加入、字节级自描述。
参考了 QRFerry(QF4) / Decimen 的公开设计思路，**不复用其任何代码**（上游仓库无开源许可）。

## 0. 设计约束

| 约束 | 后果 |
|---|---|
| 屏幕 → 摄像头是单向有损信道，无反向通道 | 不能请求重传；必须喷泉码 |
| 摄像头帧率与屏幕刷新不同步、滚动快门撕裂 | 帧必须自描述；双通道交替刷新 |
| 接收端可能中途加入 | 每帧都要能重建解码器参数 |
| 视距内任何人可拍屏 | 加密必须在传输层之外解决（M2 加密封封） |

## 1. 容器 SMC1（RaptorQ 保护的对象）

```
offset  size  field
0       4     magic "SMC1"
4       1     version = 1
5       1     comp: 0=raw 1=gzip 2=brotli
6       2     name_len  (LE)
8       n     name (UTF-8)
8+n     8     orig_len  (LE, 原始文件字节数)
16+n    4     orig_crc  (LE, 原始文件 CRC-32)
20+n    ...   payload   (压缩后或原始字节)
```

元数据在受保护对象内部：接收端只有还原整个对象才能拿到文件名，
不需要独立的描述帧，也不存在「信标帧被漏掉就收不到文件名」的问题。

## 2. 帧 SMQ（每帧 = 1 个 RaptorQ 编码符号）

```
offset  size  field
0       3     magic "SMQ"
3       1     version = 1
4       4     session      = CRC-32(container)，兼作对象校验
8       4     object_len   = 容器长度 F（RaptorQ OTI 的 F）
12      2     symbol_size  = 符号大小 T（RaptorQ OTI 的 T）
14      1     sbn          源块号
15      3     esi          编码符号 ID（24 bit，LE）
18      T     payload      一个 RaptorQ 编码符号
18+T    4     crc32        覆盖前面所有字节
```

固定开销 **22 字节/帧**。每帧自描述：接收端拿到任意一帧就能构造
`ObjectTransmissionInformation::with_defaults(F, T)` 并开始解码。

RaptorQ 参数：使用 `raptorq` crate 的 `with_defaults(F, T)`，收发两端用同样的
(F, T) 派生 Z/N/Al，**不得**单侧改动（会导致解码失败）。

## 3. 发送端流程

1. 读文件 → `compress::best()`（gzip-9 / brotli-q11 择优，只在省字节时用）
2. `container::encode()` → 容器
3. RaptorQ 编码：源符号 + `ceil(K * repair_pct/100)` 个修复符号
4. **跨源块轮转交织**：连续丢失不会打掉同一个块的全部符号
5. 逐帧序列化 → 无限循环播放（无 ACK，发送端不感知进度）

## 4. 接收端流程

1. 摄像头取帧 → QR 解码（ZXing）
2. CRC-32 校验失败即丢弃（模糊/撕裂帧）
3. 按 (session, sbn, esi) 去重
4. 喂给 RaptorQ 解码器，凑够任意 K' 个不同符号即还原
5. 四层校验：
   - 帧 CRC-32
   - 对象 CRC-32（= session）
   - 解压后长度 == orig_len
   - 原文件 CRC-32 == orig_crc

## 5. 档位（以实测容量为准，见 `sendmecongo-bench capacity`）

| preset | QR | 通道 | 符号/秒 | 保持刷新 | 修复比例 |
|---|---|---|---|---|---|
| robust | V15-L | 1 | 10 | 6 | 35% |
| balanced | V20-L | 1 | 15 | 4 | 25% |
| turbo15 | V30-L | 1 | 15 | 4 | 20% |
| turbo30 | V30-L | 1 | 30 | 2 | 20% |
| turbo60 | V30-L | 2（交替） | 60 | 2 | 20% |
| megabit | V40-L | 2（交替） | 60 | 2 | 20% |

双通道：两块二维码左右并排，交替更新，任意时刻一块保持稳定以规避滚动快门撕裂。

## 6. 尚未实现（M2/M3）

- 加密封封层：AES-256-GCM + Argon2id，位于容器之前，不破坏本格式
- 审计日志
- 多文件批次与分卷
- 录像/PNG 序列离线解析通道
