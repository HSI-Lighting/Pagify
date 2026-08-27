import SwiftUI

/// Pick any colour, from a wheel. Android's `ui/components/ColourWheel.kt`.
///
/// A wheel rather than three sliders because the question being asked is "which
/// colour", not "how much red": hue around, saturation outward, and one slider for
/// brightness. Someone reaching for this has a colour in mind and wants to point at
/// it.
///
/// The palette beside it covers the common cases; this is for the ones it does not,
/// which is why it is presented over the ribbon rather than being another row of
/// controls always on screen.
struct ColourWheelDialog: View {
    let onPick: (MarkColor) -> Void
    let onDismiss: () -> Void

    @State private var hue: CGFloat
    @State private var saturation: CGFloat
    @State private var value: CGFloat
    @Environment(\.colorScheme) private var scheme

    /// The colour in hand decides where the marker starts, and nothing after that.
    /// It is taken apart here rather than held, because the wheel's state is hue,
    /// saturation and brightness — a stored packed colour beside them is a second
    /// answer to the same question, free to disagree with the first.
    init(initial: MarkColor, onPick: @escaping (MarkColor) -> Void,
         onDismiss: @escaping () -> Void) {
        self.onPick = onPick
        self.onDismiss = onDismiss
        let start = HSV(colour: initial)
        _hue = State(initialValue: start.hue)
        _saturation = State(initialValue: start.saturation)
        _value = State(initialValue: start.value)
    }

    private var picked: HSV { HSV(hue: hue, saturation: saturation, value: value) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Pick a colour")
                .font(.title3.weight(.semibold))
                .foregroundStyle(PagifyColor.onSurface(scheme))

            Wheel(hue: $hue, saturation: $saturation, value: value)
                .aspectRatio(1, contentMode: .fit)
                .padding(.horizontal, 12)

            BrightnessStrip(hue: hue, saturation: saturation, value: $value)
                .frame(height: 36)
                .padding(.horizontal, 12)

            HStack(spacing: 12) {
                Circle()
                    .fill(Color(picked.markColor.cgColor))
                    .frame(width: 36, height: 36)
                // The hex is not decoration: it is how a colour gets matched to one
                // already in a document, or written down.
                Text(picked.hex)
                    .font(.body)
                    .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
            }

            // Typed, for when the colour is already known.
            //
            // A wheel is for finding a colour you are looking at. It is no use for
            // matching one you have been handed as three numbers, which is how a
            // house colour or a drawing standard arrives. These drive the same hue
            // and saturation the wheel does, so the marker moves to what is typed
            // and the two can never disagree.
            RgbFields(channels: picked.quantisedChannels) { red, green, blue in
                // Back through the packed integer, not straight from the floats:
                // that is the trip a typed number takes on Android, and it is what
                // makes 128 stay 128 rather than drifting to 127 on the next edit.
                let hsv = HSV(colour: MarkColor(r: Int(red * 255), g: Int(green * 255),
                                                b: Int(blue * 255), a: 255))
                hue = hsv.hue
                saturation = hsv.saturation
                value = hsv.value
            }

            HStack {
                Spacer()
                Button("Cancel", action: onDismiss)
                Button("Use this colour") { onPick(picked.markColor) }
                    .fontWeight(.semibold)
            }
        }
        .padding(20)
        .background(PagifyColor.surface(scheme))
    }
}

/// Hue around, saturation outward.
///
/// Drawn as wedges rather than a shader: a sweep gradient gives the hues but not
/// the saturation falloff, and a per-pixel bitmap would be rebuilt on every
/// redraw. A few hundred anti-aliased wedges are indistinguishable from a
/// continuous wheel at this size and cost nothing to draw again.
private struct Wheel: View {
    @Binding var hue: CGFloat
    @Binding var saturation: CGFloat
    let value: CGFloat

    @Environment(\.displayScale) private var displayScale
    /// The measured side, kept because a `Canvas` tells only its drawing closure
    /// how big it is, and the gesture has to agree with it about where the centre
    /// of the wheel is.
    @State private var side: CGFloat = 1

    var body: some View {
        Canvas { context, size in
            let radius = min(size.width, size.height) / 2
            let centre = CGPoint(x: size.width / 2, y: size.height / 2)

            for step in 0..<wedges {
                let from = CGFloat(step) * 360 / CGFloat(wedges)
                var wedge = Path()
                wedge.move(to: centre)
                wedge.addArc(center: centre, radius: radius,
                             startAngle: .degrees(from - 0.5),
                             // A hair of overlap, or the seams between wedges show
                             // as spokes.
                             endAngle: .degrees(from - 0.5 + 360 / CGFloat(wedges) + 1),
                             clockwise: false)
                wedge.closeSubpath()
                context.fill(wedge, with: .radialGradient(
                    Gradient(colors: [
                        HSV(hue: from, saturation: 0, value: value).colour,
                        HSV(hue: from, saturation: 1, value: value).colour,
                    ]),
                    center: centre, startRadius: 0, endRadius: radius))
            }

            // Where the current colour sits, so the wheel shows a state rather than
            // just offering a choice.
            let marker = CGPoint(x: centre.x + cos(hue * .pi / 180) * saturation * radius,
                                 y: centre.y + sin(hue * .pi / 180) * saturation * radius)
            let ring = Path(ellipseIn: CGRect(x: marker.x - markerRadius(displayScale),
                                              y: marker.y - markerRadius(displayScale),
                                              width: markerRadius(displayScale) * 2,
                                              height: markerRadius(displayScale) * 2))
            context.stroke(ring, with: .color(.white), lineWidth: 4 / displayScale)
            context.stroke(ring, with: .color(.black), lineWidth: 2 / displayScale)
        }
        .background(GeometryReader { geometry in
            Color.clear.preference(key: WheelSideKey.self,
                                   value: min(geometry.size.width, geometry.size.height))
        })
        .onPreferenceChange(WheelSideKey.self) { side = max($0, 1) }
        .contentShape(Rectangle())
        // A zero minimum distance so a tap works as well as a drag, because both
        // are natural here: a tap to jump to a colour, a drag to hunt for one.
        .gesture(DragGesture(minimumDistance: 0).onChanged(report))
    }

    private func report(_ drag: DragGesture.Value) {
        let radius = side / 2
        let delta = CGPoint(x: drag.location.x - radius, y: drag.location.y - radius)

        hue = (atan2(delta.y, delta.x) * 180 / .pi + 360).truncatingRemainder(dividingBy: 360)
        // Clamped rather than ignored past the edge: a finger that slides off the
        // wheel while dragging should hold the outer colour, not drop the gesture.
        saturation = min(max(hypot(delta.x, delta.y) / radius, 0), 1)
    }
}

private struct WheelSideKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

/// Black to the full-strength colour, which is what "brightness" means here.
private struct BrightnessStrip: View {
    let hue: CGFloat
    let saturation: CGFloat
    @Binding var value: CGFloat

    @Environment(\.displayScale) private var displayScale
    @State private var width: CGFloat = 1

    var body: some View {
        Canvas { context, size in
            let track = Path(roundedRect: CGRect(origin: .zero, size: size),
                             cornerRadius: 18 / displayScale)
            context.fill(track, with: .linearGradient(
                Gradient(colors: [HSV(hue: hue, saturation: saturation, value: 0).colour,
                                  HSV(hue: hue, saturation: saturation, value: 1).colour]),
                startPoint: .zero, endPoint: CGPoint(x: size.width, y: 0)))

            let x = value * size.width
            let knob = 12 / displayScale
            let dot = Path(ellipseIn: CGRect(x: x - knob, y: size.height / 2 - knob,
                                             width: knob * 2, height: knob * 2))
            context.fill(dot, with: .color(.white))
            context.stroke(dot, with: .color(.black.opacity(0.4)), lineWidth: 2 / displayScale)
        }
        .background(GeometryReader { geometry in
            Color.clear.preference(key: StripWidthKey.self, value: geometry.size.width)
        })
        .onPreferenceChange(StripWidthKey.self) { width = max($0, 1) }
        .contentShape(Rectangle())
        .gesture(DragGesture(minimumDistance: 0).onChanged { drag in
            value = min(max(drag.location.x / width, 0), 1)
        })
    }
}

private struct StripWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

/// Red, green and blue, as three numbers you can type.
///
/// Each field holds its own text while it is being edited rather than being
/// re-derived from the colour on every keystroke: clearing a field to type a new
/// number leaves it momentarily empty, and a field that rewrote itself to "0" the
/// instant you deleted the last digit would be impossible to type in.
///
/// Out-of-range and half-typed values are simply not applied. Nothing is rejected
/// or flagged — the colour just does not move until the number makes sense.
private struct RgbFields: View {
    let channels: (CGFloat, CGFloat, CGFloat)
    let onChannels: (CGFloat, CGFloat, CGFloat) -> Void

    var body: some View {
        HStack(spacing: 8) {
            // **Rounded** on the way out, where the packed colour truncates on the
            // way in. Both are Android's, and they are not the same operation:
            // showing a truncated 127 for a channel the wheel is holding at 127.6
            // is what makes a typed number appear to change on its own.
            ChannelField(label: "R", current: shown(channels.0)) {
                onChannels(CGFloat($0) / 255, channels.1, channels.2)
            }
            ChannelField(label: "G", current: shown(channels.1)) {
                onChannels(channels.0, CGFloat($0) / 255, channels.2)
            }
            ChannelField(label: "B", current: shown(channels.2)) {
                onChannels(channels.0, channels.1, CGFloat($0) / 255)
            }
        }
    }

    private func shown(_ channel: CGFloat) -> Int { Int((channel * 255).rounded()) }
}

private struct ChannelField: View {
    let label: String
    let current: Int
    let onValue: (Int) -> Void

    @State private var text: String = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption).foregroundStyle(.secondary)
            TextField(label, text: $text)
                .keyboardType(.numberPad)
                .textFieldStyle(.roundedBorder)
                .onChange(of: text) { _, typed in
                    let digits = String(typed.filter(\.isNumber).prefix(3))
                    if digits != typed { text = digits }
                    guard let entered = Int(digits), (0...255).contains(entered) else { return }
                    onValue(entered)
                }
        }
        // Keyed on what came in, so a colour changed on the wheel rewrites the
        // field — but only when the number itself moved, never mid-keystroke.
        .onAppear { text = String(current) }
        .onChange(of: current) { _, now in
            if Int(text) != now { text = String(now) }
        }
    }
}

/// Hue in degrees, saturation and value 0…1 — Android's `android.graphics.Color`
/// HSV, reimplemented because there is no system equivalent that round-trips the
/// same way.
struct HSV {
    var hue: CGFloat
    var saturation: CGFloat
    var value: CGFloat

    init(hue: CGFloat, saturation: CGFloat, value: CGFloat) {
        self.hue = hue
        self.saturation = saturation
        self.value = value
    }

    init(colour: MarkColor) {
        let r = CGFloat(colour.r) / 255
        let g = CGFloat(colour.g) / 255
        let b = CGFloat(colour.b) / 255
        let high = max(r, g, b)
        let low = min(r, g, b)
        let span = high - low

        value = high
        saturation = high <= 0 ? 0 : span / high
        if span <= 0 {
            hue = 0
        } else if high == r {
            hue = (60 * ((g - b) / span) + 360).truncatingRemainder(dividingBy: 360)
        } else if high == g {
            hue = 60 * ((b - r) / span) + 120
        } else {
            hue = 60 * ((r - g) / span) + 240
        }
    }

    var colour: Color { Color(markColor.cgColor) }

    /// **Truncating**, not rounding. This is Android's `toArgbLong`, and it is what
    /// decides whether a colour picked here is the same integer as the palette
    /// swatch it was dragged onto — rounding would put the two one apart and the
    /// swatch would stop reading as selected.
    var markColor: MarkColor {
        let (r, g, b) = channels
        return MarkColor(r: Int(r * 255), g: Int(g * 255), b: Int(b * 255), a: 255)
    }

    var hex: String {
        let colour = markColor
        return String(format: "#%02X%02X%02X", colour.r, colour.g, colour.b)
    }

    /// The colour as three fractions, before anything packs it into bytes.
    /// The channels as the committed colour will actually hold them.
    ///
    /// The readout and the swatch have to quantise through the same byte the
    /// colour does, or they disagree by one: a brightness of exactly 0.5 shows
    /// 128 in the field while the hex reads 7F and the committed value is 127.
    var quantisedChannels: (CGFloat, CGFloat, CGFloat) {
        let packed = markColor
        return (CGFloat(packed.r) / 255, CGFloat(packed.g) / 255, CGFloat(packed.b) / 255)
    }

    var channels: (CGFloat, CGFloat, CGFloat) {
        let h = hue.truncatingRemainder(dividingBy: 360) / 60
        let sector = Int(floor(h)) % 6
        let fraction = h - floor(h)
        let p = value * (1 - saturation)
        let q = value * (1 - saturation * fraction)
        let t = value * (1 - saturation * (1 - fraction))

        switch sector < 0 ? sector + 6 : sector {
        case 0: return (value, t, p)
        case 1: return (q, value, p)
        case 2: return (p, value, t)
        case 3: return (p, q, value)
        case 4: return (t, p, value)
        default: return (value, p, q)
        }
    }
}

/// Wedges the wheel is drawn from. 180 is two degrees each, which is past the point
/// where a seam is visible at any size this can be shown at.
private let wedges = 180

/// How big the marker ring is, in Android's device pixels — divided by the screen's
/// scale so it comes out the same size here rather than three times too big.
private func markerRadius(_ displayScale: CGFloat) -> CGFloat { 14 / displayScale }
