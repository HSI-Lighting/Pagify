import UIKit

/// HEIC/JPEG/PNG → an upright, opaque RGBA image, ready for Vision.
///
/// Two things Vision itself will not do: apply EXIF orientation (a sideways
/// photo stays sideways, joins stay wrong-way-round) and bound the pixel
/// count (an uncapped HEIC from a modern phone's camera is tens of megapixels
/// — plenty to make Vision's own memory use the thing that gets the app
/// killed, not a bug in Vision).
enum CardImage {
    /// Matches the ceiling the PDF engine itself renders under — one number
    /// this app already treats as "as large as a photo needs to be."
    static let maxDimension: CGFloat = 16_384
    static let maxPixels: CGFloat = 64 * 1024 * 1024

    /// `nil` if the data does not decode as an image at all.
    static func load(_ data: Data) -> UIImage? {
        guard let raw = UIImage(data: data) else { return nil }

        // `UIImage(data:)` already reads EXIF into `.imageOrientation`, but
        // that orientation is a property of the *view*, not the pixels —
        // Vision and CGImage-based code (this file's own caller) look at the
        // pixels directly and never consult it. Redrawing through
        // `UIGraphicsImageRenderer` bakes the rotation into the bitmap once,
        // here, so nothing downstream has to remember to ask.
        let upright = redrawUpright(raw)
        return withinLimits(upright.size.width, upright.size.height)
            ? upright
            : downscaled(upright)
    }

    private static func redrawUpright(_ image: UIImage) -> UIImage {
        guard image.imageOrientation != .up else { return image }
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let renderer = UIGraphicsImageRenderer(size: image.size, format: format)
        return renderer.image { context in
            // Opaque white first: a transparent HEIC composited onto whatever
            // was previously in the buffer reads its old contents through as
            // "text" to Vision.
            UIColor.white.setFill()
            context.fill(CGRect(origin: .zero, size: image.size))
            image.draw(in: CGRect(origin: .zero, size: image.size))
        }
    }

    /// True when a photo is already inside both the per-side and total-pixel
    /// ceilings — extracted as a pure, testable function rather than inlined,
    /// since the two-part rule (a dimension check AND an area check) is
    /// exactly the kind of thing worth asserting against directly.
    static func withinLimits(_ width: CGFloat, _ height: CGFloat) -> Bool {
        width <= maxDimension && height <= maxDimension && width * height <= maxPixels
    }

    private static func downscaled(_ image: UIImage) -> UIImage {
        let width = image.size.width
        let height = image.size.height
        let byDimension = min(maxDimension / width, maxDimension / height, 1)
        let byArea = min((maxPixels / (width * height)).squareRoot(), 1)
        let scale = min(byDimension, byArea)

        let size = CGSize(width: (width * scale).rounded(.down), height: (height * scale).rounded(.down))
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in
            image.draw(in: CGRect(origin: .zero, size: size))
        }
    }
}
