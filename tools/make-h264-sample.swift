// 合成一段极小的 H.264 测试码流（testdata/h264-sample.mp4）。
//
// 为什么需要它：`recordings/` 里的相机素材全是 iPhone 的 HEVC（`hvc1`），
// 所以 `cargo test` 长期只覆盖 `rusty_h265` 那一半；openh264 那一半连一次
// 真实解码都没有过。测试要能长期挡住 H.264 路径的回归，就必须有一段能入库的
// 小码流 —— 相机素材单条 56~312 MB，进不了 git。
//
// 画面是灰度阶梯：第 i 帧整体填充 `i * 255 / (frames-1)`。解码回来只要亮度
// 严格递增、首尾拉开，就说明帧没丢、没重、没错位，而且 luma 抽样是对的。
//
// 用法（macOS 自带 AVFoundation，不需要 ffmpeg / 第三方库）：
//   swift tools/make-h264-sample.swift testdata/h264-sample.mp4
//
// 生成物已随仓库跟踪；只有在需要改这段代码流时才重跑。

import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation

let width = 320
let height = 240
let frames = 8
let fps: Int32 = 30
// 每 4 帧一个关键帧：8 帧正好两个 GOP，让「参数集在随机访问点前重发」这条
// 逻辑也有东西可测，而不是退化成每帧都带 SPS/PPS。
let keyframeInterval = 4

guard CommandLine.arguments.count >= 2 else {
    FileHandle.standardError.write("用法: swift make-h264-sample.swift <输出.mp4>\n".data(using: .utf8)!)
    exit(2)
}
let output = URL(fileURLWithPath: CommandLine.arguments[1])
try? FileManager.default.removeItem(at: output)
try? FileManager.default.createDirectory(
    at: output.deletingLastPathComponent(), withIntermediateDirectories: true)

let writer = try AVAssetWriter(outputURL: output, fileType: .mp4)
let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
    AVVideoCodecKey: AVVideoCodecType.h264,
    AVVideoWidthKey: width,
    AVVideoHeightKey: height,
    AVVideoCompressionPropertiesKey: [
        AVVideoAverageBitRateKey: 400_000,
        AVVideoMaxKeyFrameIntervalKey: keyframeInterval,
    ],
])
input.expectsMediaDataInRealTime = false

let adaptor = AVAssetWriterInputPixelBufferAdaptor(
    assetWriterInput: input,
    sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: width,
        kCVPixelBufferHeightKey as String: height,
    ])
writer.add(input)
guard writer.startWriting() else {
    FileHandle.standardError.write("startWriting 失败: \(writer.error?.localizedDescription ?? "?")\n".data(using: .utf8)!)
    exit(1)
}
writer.startSession(atSourceTime: .zero)

let space = CGColorSpaceCreateDeviceRGB()
for index in 0..<frames {
    let level = Double(index) * 255.0 / Double(frames - 1) / 255.0
    guard let pool = adaptor.pixelBufferPool else {
        FileHandle.standardError.write("没有 pixel buffer pool\n".data(using: .utf8)!)
        exit(1)
    }
    var buffer: CVPixelBuffer?
    guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &buffer) == kCVReturnSuccess,
        let pixelBuffer = buffer
    else {
        FileHandle.standardError.write("拿不到 pixel buffer\n".data(using: .utf8)!)
        exit(1)
    }
    CVPixelBufferLockBaseAddress(pixelBuffer, [])
    guard let context = CGContext(
        data: CVPixelBufferGetBaseAddress(pixelBuffer),
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: CVPixelBufferGetBytesPerRow(pixelBuffer),
        space: space,
        bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)
    else {
        FileHandle.standardError.write("CGContext 建不起来\n".data(using: .utf8)!)
        exit(1)
    }
    context.setFillColor(CGColor(red: level, green: level, blue: level, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    CVPixelBufferUnlockBaseAddress(pixelBuffer, [])

    // 缓冲池有限，必须等输入真的吃下这一帧再要下一个。
    while !input.isReadyForMoreMediaData {
        Thread.sleep(forTimeInterval: 0.01)
    }
    if !adaptor.append(pixelBuffer, withPresentationTime: CMTime(value: Int64(index), timescale: fps)) {
        FileHandle.standardError.write("append 第 \(index) 帧失败: \(writer.error?.localizedDescription ?? "?")\n".data(using: .utf8)!)
        exit(1)
    }
}

input.markAsFinished()
let done = DispatchSemaphore(value: 0)
writer.finishWriting { done.signal() }
done.wait()

if writer.status != .completed {
    FileHandle.standardError.write("写入失败: \(writer.error?.localizedDescription ?? "?")\n".data(using: .utf8)!)
    exit(1)
}
let attrs = try? FileManager.default.attributesOfItem(atPath: output.path)
let size = (attrs?[.size] as? Int) ?? 0
print("已写出 \(output.path) · \(frames) 帧 · \(width)×\(height) · \(size) 字节")
