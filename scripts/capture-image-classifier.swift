import CoreGraphics
import Foundation
import ImageIO

func uniform(_ rgba: [UInt8], width: Int, height: Int) -> Bool {
    var background: (UInt8, UInt8, UInt8)?
    for y in 0..<height {
        for x in 0..<width {
            let offset = (y * width + x) * 4
            if rgba[offset + 3] < 13 { continue }
            guard let first = background else {
                background = (rgba[offset], rgba[offset + 1], rgba[offset + 2])
                continue
            }
            if abs(Int(rgba[offset]) - Int(first.0)) > 5
                || abs(Int(rgba[offset + 1]) - Int(first.1)) > 5
                || abs(Int(rgba[offset + 2]) - Int(first.2)) > 5 {
                return false
            }
        }
    }
    return true
}

func rgbaPixels(_ image: CGImage) -> [UInt8]? {
    let width = image.width
    let height = image.height
    var rgba = [UInt8](repeating: 0, count: width * height * 4)
    let drawn = rgba.withUnsafeMutableBytes { buffer in
        guard let context = CGContext(
            data: buffer.baseAddress,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return false }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return true
    }
    return drawn ? rgba : nil
}

if CommandLine.arguments.last == "--self-test" {
    var pixels = [UInt8](repeating: 255, count: 32 * 32 * 4)
    precondition(uniform(pixels, width: 32, height: 32), "a flat image must be classified as blank")
    let center = (16 * 32 + 16) * 4
    pixels[center] = 0
    pixels[center + 1] = 0
    pixels[center + 2] = 0
    precondition(!uniform(pixels, width: 32, height: 32), "nonuniform pixels must require review")
    print("capture image classifier self-check passed")
} else {
    guard CommandLine.arguments.count == 2,
          let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: CommandLine.arguments[1]) as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
          let pixels = rgbaPixels(image) else {
        fatalError("expected a readable PNG path")
    }
    print(uniform(pixels, width: image.width, height: image.height) ? "blank" : "review")
}
