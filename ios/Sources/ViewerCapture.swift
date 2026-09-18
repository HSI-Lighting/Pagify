import CoreGraphics
import Foundation
import SwiftUI
import UIKit

/// Taking a picture of a STEP model or a DXF/DWG drawing — the iOS twin of
/// Android's `ModelCapture.kt`. Genuinely shared between the two viewers,
/// the way it is shared on Android: sizing, the crop/mask arithmetic and the
/// file name all come from here, so a model and a drawing produce the same
/// kind of picture and one change fixes both.
///
/// **Drawn again, not grabbed off the screen.** The picture is re-rendered by
/// the engine from the camera or the sheet the viewer is showing, so it
/// contains the part or the drawing and nothing else — no ribbon, no details
/// panel, no banner that happened to be up. Those pixels are never in it
/// because they are never drawn.
///
/// It also means the picture need not be the size of the screen: `scale`
/// asks for it larger, up to `mostPixels`.

/// The ribbon's capture tool. Matches Android's own `ModelRibbon` exactly —
/// one "Snapshot" button whose tap opens a menu choosing Box or Ring, not two
/// separately armed tools — reused by both viewers the same way `ModelRibbon`
/// itself is. Android's own doc explains the shape: "the button means
/// 'snapshot' and the menu means 'which shape', exactly as on a page: the
/// shape is only ever chosen after deciding to take one, never before."
///
/// The button's own icon follows whichever shape is currently armed (a ring
/// once Ring is chosen, a box otherwise) and a checkmark marks the armed
/// choice in the menu, the same as Android's `DropdownMenuItem` trailing
/// icon.
struct SnapshotMenuButton: View {
    @Binding var framing: Bool
    @Binding var lasso: Bool

    var body: some View {
        Menu {
            Button {
                // Choosing the shape already armed puts the tool away, so
                // this stays a toggle rather than a one-way switch — matches
                // Android's own `onFrame(!(framing && !lasso))`.
                let holding = framing && !lasso
                lasso = false
                framing = !holding
            } label: {
                if framing && !lasso {
                    Label("Box", systemImage: "checkmark")
                } else {
                    Text("Box")
                }
            }
            Button {
                let holding = framing && lasso
                lasso = true
                framing = !holding
            } label: {
                if framing && lasso {
                    Label("Ring", systemImage: "checkmark")
                } else {
                    Text("Ring")
                }
            }
        } label: {
            Label("Snapshot", systemImage: framing && lasso ? "scribble.variable" : "viewfinder")
                .labelStyle(.iconOnly)
                .padding(14)
        }
        .buttonStyle(.borderedProminent)
        .tint(framing ? .orange : .accentColor)
        .clipShape(Circle())
    }
}

/// Twelve megapixels: a 48 MB bitmap, held briefly while it is encoded.
/// Matches Android's own `MOST_CAPTURE_PIXELS` exactly — chosen so it does
/// not defeat the ordinary case (twice a 1179-wide phone screen is already
/// past 8 megapixels) while still bounding a tablet at four times size.
let mostViewerCapturePixels = 12_000_000

/// How large a capture to draw, in pixels. The view's own proportions are
/// kept exactly, so the picture frames the part the way the screen does.
func viewerCaptureSize(viewWidth: Int, viewHeight: Int, scale: Int,
                       mostPixels: Int = mostViewerCapturePixels) -> (width: Int, height: Int) {
    guard viewWidth > 0, viewHeight > 0 else { return (0, 0) }
    let wide = viewWidth * max(1, scale)
    let high = viewHeight * max(1, scale)
    let pixels = wide * high
    if pixels <= mostPixels { return (wide, high) }

    // Rounded down, not to nearest: rounding both sides up puts the result
    // back over the ceiling it was brought under.
    let shrink = (Double(mostPixels) / Double(pixels)).squareRoot()
    return (max(viewWidth, Int(Double(wide) * shrink)),
            max(viewHeight, Int(Double(high) * shrink)))
}

/// Where a region dragged on screen lands in the re-rendered picture. Scaled
/// by the same factor in both directions and clipped to the picture — a drag
/// that ran off the edge of the view, which is how anyone selects something
/// against the border, would otherwise ask for pixels that were never drawn.
/// `nil` when nothing usable is left.
func viewerRegionInCapture(box: PageRect, viewWidth: Int, viewHeight: Int,
                          wholeWidth: Int, wholeHeight: Int) -> CGRect? {
    guard viewWidth > 0, viewHeight > 0, wholeWidth > 0, wholeHeight > 0 else { return nil }
    let across = CGFloat(wholeWidth) / CGFloat(viewWidth)
    let down = CGFloat(wholeHeight) / CGFloat(viewHeight)

    let left = min(max(Int(box.left * across), 0), wholeWidth)
    let top = min(max(Int(box.top * down), 0), wholeHeight)
    let right = min(max(Int((box.right * across).rounded()), 0), wholeWidth)
    let bottom = min(max(Int((box.bottom * down).rounded()), 0), wholeHeight)

    guard right - left >= 1, bottom - top >= 1 else { return nil }
    return CGRect(x: left, y: top, width: right - left, height: bottom - top)
}

/// Cut the region out, blanking anything the lasso left outside itself —
/// matches Android's `cutOut`. `ring` is in the same view points `box` was
/// dragged in; `factor` is how much bigger the capture is than the view
/// (`wholeWidth / viewWidth`), which is what turns those points into the
/// crop's own pixels.
///
/// A box (fewer than 3 ring points) is just the crop. A ring is drawn into a
/// **fresh** image through the traced path rather than erased out of the
/// crop: `CGContext` has no reliable "erase to transparent" that survives
/// every colour space, and starting from blank cannot leave a fringe.
func viewerCutOut(_ whole: CGImage, cut: CGRect, ring: [CGPoint], factor: CGFloat) -> CGImage {
    guard let cropped = whole.cropping(to: cut) else { return whole }
    guard ring.count >= 3 else { return cropped }

    let size = CGSize(width: cropped.width, height: cropped.height)
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    format.opaque = false
    let renderer = UIGraphicsImageRenderer(size: size, format: format)

    // `UIGraphicsImageRenderer` draws in UIKit's own coordinate space —
    // origin top-left, y down — which is exactly the space `ring`'s points
    // (and `cut`'s) already live in, so nothing here needs flipping the way
    // a raw `CGContext` would.
    let image = renderer.image { _ in
        let path = UIBezierPath()
        for (index, point) in ring.enumerated() {
            let mapped = CGPoint(x: point.x * factor - cut.minX, y: point.y * factor - cut.minY)
            if index == 0 { path.move(to: mapped) } else { path.addLine(to: mapped) }
        }
        path.close()
        path.addClip()
        UIImage(cgImage: cropped).draw(in: CGRect(origin: .zero, size: size))
    }
    return image.cgImage ?? cropped
}

/// What a capture is called once it is a file. The source file's own name
/// first, because a folder of captures is sorted by name and a timestamp
/// alone says nothing about which part or drawing it is.
func viewerCaptureFileName(source: String, stamp: String, format: CaptureFormat) -> String {
    let allowed = Set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 _-")
    let kept = (source as NSString).deletingPathExtension
        .filter { allowed.contains($0) }
        .trimmingCharacters(in: .whitespaces)
    let stem = String(kept.prefix(48))
    return "\(stem.isEmpty ? "Pagify" : stem) \(stamp).\(format.fileExtension)"
}

/// Burn committed marks into a picture, through the same engine code a page
/// capture uses (`render::markup::composite`) — see `pagify_composite_markup_into`'s
/// own doc for why this needs no document or page. Returns `image` unchanged
/// if there is nothing to draw or the engine call fails; a capture that could
/// not be marked up is still a capture.
func compositeViewerMarkup(into image: CGImage, marks: [Markup], scale: CGFloat) -> CGImage {
    guard !marks.isEmpty else { return image }

    let width = image.width, height = image.height
    let bitmapInfo = CGImageAlphaInfo.premultipliedLast.rawValue
        | CGBitmapInfo.byteOrder32Big.rawValue
    guard let context = CGContext(
        data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: bitmapInfo
    ), let pixels = context.data else { return image }

    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))

    let json = markupWireJSON(marks)
    let outcome = json.withCString { text in
        pagify_composite_markup_into(
            UInt32(width), UInt32(height),
            pixels.assumingMemoryBound(to: UInt8.self),
            context.bytesPerRow, Float(scale), text)
    }
    guard outcome == PAGIFY_OK, let marked = context.makeImage() else { return image }
    return marked
}

/// A picture as bytes, ready to save, share or copy.
func encodeViewerCapture(_ image: CGImage, format: CaptureFormat, quality: CGFloat = 0.92) -> Data? {
    let picture = UIImage(cgImage: image)
    switch format {
    case .png: return picture.pngData()
    case .jpeg: return picture.jpegData(compressionQuality: quality)
    }
}

/// A fixed timestamp pattern, matching `ReaderView`'s own capture file names.
func viewerCaptureTimestamp() -> String {
    let formatter = DateFormatter()
    formatter.dateFormat = "yyyy-MM-dd HHmmss"
    return formatter.string(from: Date())
}
