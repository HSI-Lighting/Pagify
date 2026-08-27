// Build the iOS app icon from the Android adaptive icon's two layers.
//
// Run from the repo root when the Android artwork changes:
//
//   swift ios/Scripts/make-appicon.swift \
//       app/src/main/res/drawable-xxxhdpi/ic_launcher_foreground.png \
//       ios/Assets.xcassets/AppIcon.appiconset/icon-1024.png
//
// Generated rather than hand-exported so the two platforms cannot drift: there
// is one piece of artwork, and this is the arithmetic that adapts it.
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

// values/ic_launcher_background.xml
let background = (r: 0x3B / 255.0, g: 0x00 / 255.0, b: 0xE6 / 255.0)
let side = 1024

guard CommandLine.arguments.count == 3 else {
    FileHandle.standardError.write(Data("usage: make-appicon.swift <foreground.png> <out.png>\n".utf8))
    exit(2)
}
let input = URL(fileURLWithPath: CommandLine.arguments[1])
let output = URL(fileURLWithPath: CommandLine.arguments[2])

guard let source = CGImageSourceCreateWithURL(input as CFURL, nil),
      let foreground = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
    FileHandle.standardError.write(Data("could not read \(input.path)\n".utf8))
    exit(1)
}

// An adaptive icon's layers are a 108dp canvas of which only the central 72dp is
// ever shown — the outer ring is bleed for the launcher's parallax and for masks
// of different shapes. iOS has no such reserve: its superellipse is inscribed in
// the whole square. So the 72dp region is what maps onto the full iOS icon, and
// using the entire 108dp canvas instead would render the mark two-thirds the
// size it is on Android.
let visible = 72.0 / 108.0
let cropSide = Int((Double(foreground.width) * visible).rounded())
let inset = (foreground.width - cropSide) / 2

guard let cropped = foreground.cropping(to: CGRect(
    x: inset, y: inset, width: cropSide, height: cropSide
)) else {
    FileHandle.standardError.write(Data("could not crop the foreground\n".utf8))
    exit(1)
}

// `noneSkipLast`: an iOS app icon must carry no alpha at all — a transparent one
// is rejected at submission, and the rounding is the system's job, not ours.
guard let context = CGContext(
    data: nil, width: side, height: side,
    bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceRGB(),
    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
) else {
    FileHandle.standardError.write(Data("could not allocate the canvas\n".utf8))
    exit(1)
}

// Built in the context's own colour space, not with the convenience
// initialiser: that one makes an sRGB colour, and drawing it into a DeviceRGB
// context converts it — #3B00E6 comes out as #4D2FEB, a visibly different
// violet. Measured by reading the corner pixel back off the written PNG, which
// is also how to check this after any change here.
let space = CGColorSpaceCreateDeviceRGB()
guard let fill = CGColor(colorSpace: space,
                         components: [background.r, background.g, background.b, 1]) else {
    FileHandle.standardError.write(Data("could not build the background colour\n".utf8))
    exit(1)
}
context.setFillColor(fill)
context.fill(CGRect(x: 0, y: 0, width: side, height: side))
context.interpolationQuality = .high
context.draw(cropped, in: CGRect(x: 0, y: 0, width: side, height: side))

guard let image = context.makeImage(),
      let destination = CGImageDestinationCreateWithURL(
        output as CFURL, UTType.png.identifier as CFString, 1, nil) else {
    FileHandle.standardError.write(Data("could not encode \(output.path)\n".utf8))
    exit(1)
}
CGImageDestinationAddImage(destination, image, nil)
guard CGImageDestinationFinalize(destination) else {
    FileHandle.standardError.write(Data("could not write \(output.path)\n".utf8))
    exit(1)
}
print("wrote \(output.path) — \(side)x\(side)")
