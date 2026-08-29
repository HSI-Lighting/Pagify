import SwiftUI

/// One tool, as the ribbon needs to know it. Android's `RibbonTool`.
///
/// Untyped on purpose. The reader marks a page and the screenshot editor marks a
/// picture, and they have separate tool enums because they can do separate things
/// — but the *row* is the same row, and a second copy of it would be the place the
/// two quietly drifted apart. `key` is whichever enum value the caller uses; the
/// ribbon only ever compares it and hands it back.
struct RibbonTool: Identifiable, Equatable {
    let key: AnyHashable
    let icon: RibbonIcon
    let name: String
    /// Whether the slot's glyph previews this one.
    ///
    /// A group can hold more than a slot can show. The ones a group is *known* for
    /// go in the preview; variations of them are still one tap away in the picker
    /// but would only crowd the row. Nothing is hidden by this — the picker always
    /// lists the whole group.
    var inPreview: Bool = true

    var id: AnyHashable { key }
}

/// The floating bands at the foot of the reader. Android's
/// `ui/components/AnnotationToolbar.kt`.
///
/// Tapping a tool selects it, and tapping the selected tool again puts the reader
/// back to plain scrolling — a tool that can only be turned on is a trap, since
/// every touch would keep drawing.
///
/// The drawing tools share one slot. Pen, line, arrow, box, circle and cloud would
/// be six slots on a ribbon that already has seven, and they are one question
/// anyway: what shape is this mark.
///
/// Two gestures, and the same two throughout: **a tap chooses, a press adjusts.**
/// Tapping the drawing slot opens the shapes; pressing it opens what they draw
/// *with*. Choosing the shape is the frequent act and the settings are the
/// occasional one, so the frequent one is the tap.
///
/// Everything is stacked in one column rather than floated at a fixed height: with
/// the parameters band there too, a palette lifted by a constant lands on top of it
/// — and, being drawn first, underneath it.
struct ToolRibbon: View {
    @Binding var settings: AnnotationSettings
    /// Restyle whatever caption is in hand, if any.
    ///
    /// Every style control has to do two things: change what the *next* mark will
    /// look like, and change the one already selected. Writing straight into the
    /// settings only does the first, which is why a caption could be taken in
    /// hand and then not actually be changed by the ribbon showing its style.
    var onRestyle: (String) -> Void = { _ in }
    /// A style slider is being dragged, or has been let go.
    var onEditing: (Bool) -> Void = { _ in }
    /// Marks on the page being read, and in the document as a whole. Only the
    /// clear menu reads them, and it says the number out loud before it wipes
    /// anything.
    var marksOnPage: Int = 0
    var marksInDocument: Int = 0
    var onClearPage: () -> Void = {}
    var onClearAll: () -> Void = {}

    @State private var showDrawPalette = false
    @State private var showClearMenu = false
    /// Whether the band of settings is showing.
    ///
    /// Asked for rather than automatic. It used to appear the moment anything was
    /// armed, which put a row of controls over the page every time a tool was
    /// picked up — for a mark that usually wants the same colour and weight as the
    /// last one.
    @State private var showParameters = false
    /// Whether the colour wheel is open, for a colour the six do not offer.
    @State private var pickingColour = false

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(spacing: 8) {
            if showDrawPalette {
                DrawingRibbon(settings: $settings,
                              // Passed through, or every text setter writes into
                              // the settings and stops there: the caption in hand
                              // keeps the font and size it was made with.
                              onRestyle: onRestyle,
                              onEditing: onEditing,
                              onPickCustomColour: { pickingColour = true },
                              onDismiss: { showDrawPalette = false })
            }
            if showClearMenu {
                ClearMenu(marksOnPage: marksOnPage,
                          marksInDocument: marksInDocument,
                          onClearPage: onClearPage,
                          onClearAll: onClearAll,
                          onDismiss: { showClearMenu = false })
            }
            // The highlighter's colours. The drawing tools carry theirs in the
            // drawing ribbon above; the highlighter is not part of that row, and
            // one colour is all it has.
            if showParameters && settings.tool == .highlight {
                MarkParameters(tool: settings.tool,
                               colour: settings.penColor,
                               onColour: { settings.penColor = $0; onRestyle("colour") },
                               onPickCustomColour: { pickingColour = true })
            }
            toolRow
        }
        .padding(.horizontal, 10)
        .padding(.bottom, 6)
        .animation(.easeInOut(duration: 0.16), value: showDrawPalette)
        .animation(.easeInOut(duration: 0.16), value: showClearMenu)
        .animation(.easeInOut(duration: 0.16), value: showParameters)
        .sheet(isPresented: $pickingColour) {
            ColourWheelDialog(initial: settings.penColor,
                              onPick: {
                                  settings.penColor = $0
                                  pickingColour = false
                              },
                              onDismiss: { pickingColour = false })
        }
    }

    private var toolRow: some View {
        HStack(spacing: 2) {
            ToolButton(icon: .system("highlighter"), label: "Highlighter",
                       selected: settings.tool == .highlight,
                       accent: settings.penColor, hasMore: true,
                       onClick: { select(toggled(.highlight)) },
                       onLongPress: { openParameters(.highlight) })

            // The armed tool's own glyph, so the ribbon says what a drag will draw
            // rather than naming the group. Its own, shorter map — deliberately not
            // `AnnotationTool.ribbonIcon`.
            ToolButton(icon: .system(CollapsedDrawingSlot.symbol(for: settings.tool)),
                       label: CollapsedDrawingSlot.label(for: settings.tool),
                       selected: settings.tool.draws,
                       accent: settings.penColor, hasMore: true,
                       onClick: {
                           // It never arms a tool itself. Tapping the group while
                           // one of its tools is armed puts the pen down; going
                           // into the palette to tap the exact tool again was the
                           // only way to stop drawing, which is a lot of aim for
                           // "I am finished".
                           showParameters = false
                           if settings.tool.draws {
                               showDrawPalette = false
                               settings.select(.none)
                           } else {
                               showDrawPalette.toggle()
                           }
                       },
                       onLongPress: {
                           openParameters(settings.tool.draws ? settings.tool : .pen)
                       })

            ToolButton(icon: .system("textformat"), label: "Note",
                       selected: settings.tool == .note,
                       onClick: { select(toggled(.note)) })

            // Distinct from the pen on purpose: both were the same glyph once, and
            // two identical pictures in one ribbon is unreadable.
            ToolButton(icon: .system("signature"), label: "Signature",
                       selected: settings.tool == .signature,
                       onClick: { select(toggled(.signature)) })

            ToolButton(icon: .drawn(.eraser), label: "Eraser",
                       selected: settings.tool == .eraser, hasMore: true,
                       onClick: { select(toggled(.eraser)) },
                       // Clearing a page or the document lives behind the eraser
                       // because that is what it means — the same action, wider.
                       // The press does not arm the tool, so a wipe is not followed
                       // by an armed eraser under your finger.
                       onLongPress: { showClearMenu = true })

            ToolButton(icon: .system("viewfinder"), label: "Snapshot",
                       selected: settings.tool == .snapshot && !settings.captureLasso,
                       onClick: {
                           let holding = settings.tool == .snapshot && !settings.captureLasso
                           settings.captureLasso = false
                           select(holding ? .none : .snapshot)
                       })

            // Its own slot rather than a shape hidden behind a press on the one
            // beside it. They are two tools by the time you are choosing: a box for
            // most things, a ring for the detail a box cannot take without its
            // neighbours.
            ToolButton(icon: .system("scribble.variable"), label: "Draw around",
                       selected: settings.tool == .snapshot && settings.captureLasso,
                       onClick: {
                           let holding = settings.tool == .snapshot && settings.captureLasso
                           settings.captureLasso = true
                           select(holding ? .none : .snapshot)
                       })
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 28))
        .shadow(color: .black.opacity(scheme == .dark ? 0.5 : 0.18), radius: 6, y: 3)
    }

    private func toggled(_ tapped: AnnotationTool) -> AnnotationTool {
        settings.tool == tapped ? .none : tapped
    }

    /// Arm a tool, and put every palette away.
    ///
    /// A palette is a way of *choosing*; once something is chosen it has nothing
    /// left to say. Closing only on its own selection is what left the pen's shapes
    /// hanging above the highlighter's colours — two bands offering two different
    /// tools, one of them already dismissed in every sense but the visible one.
    private func select(_ tool: AnnotationTool) {
        showDrawPalette = false
        showClearMenu = false
        showParameters = false
        settings.select(tool)
    }

    /// Show what a tool draws with, arming it if it is not already.
    ///
    /// Arming it is the point: the width, the colour and the line type are one set
    /// shared by every tool that draws, so opening them for a tool you are not
    /// holding would change the next mark rather than this one.
    private func openParameters(_ tool: AnnotationTool) {
        showDrawPalette = false
        showClearMenu = false
        if settings.tool != tool { settings.select(tool) }
        showParameters = true
    }
}

/// The reader's drawing tools, in the shared ribbon. Android's `DrawingRibbon.kt`.
///
/// Everything about how the row behaves lives in `MarkRibbon`; this only says which
/// tools the reader has, what they are called, and which of its own settings each
/// slot is asking about. The screenshot editor will have its own few lines saying
/// the same for its own.
struct DrawingRibbon: View {
    /// What the weight slider spans — point sizes while a text tool is held,
    /// nib widths otherwise.
    ///
    /// The ceiling is measured from a page, and a page that has not been laid out
    /// yet reports one below the six-point floor. An inverted range traps where it
    /// is built, not where it is used, so it is clamped here.
    private var widthRange: ClosedRange<CGFloat> {
        guard settings.tool.writesText else { return AnnotationMetrics.strokeSlider }
        let floor = AnnotationMetrics.textRange.lowerBound
        return floor...max(settings.textSizeCeiling, floor)
    }

    @Binding var settings: AnnotationSettings
    /// Restyle whatever caption is in hand, if any.
    ///
    /// Every style control has to do two things: change what the *next* mark will
    /// look like, and change the one already selected. Writing straight into the
    /// settings only does the first, which is why a caption could be taken in
    /// hand and then not actually be changed by the ribbon showing its style.
    var onRestyle: (String) -> Void = { _ in }
    /// A style slider is being dragged, or has been let go.
    var onEditing: (Bool) -> Void = { _ in }
    let onPickCustomColour: () -> Void
    var onDismiss: (() -> Void)?

    var body: some View {
        let writes = settings.tool.writesText

        MarkRibbon(
            groups: AnnotationTool.drawingGroups.map { group in
                group.map {
                    RibbonTool(key: $0, icon: $0.ribbonIcon, name: $0.label,
                               // The box and the ellipse are the cloud with a
                               // different ring round the words. Showing all three
                               // in one slot says nothing the cloud does not
                               // already say, and costs the other members the room
                               // to be legible.
                               inPreview: $0.inPreview)
                }
            },
            armed: settings.tool,
            colour: settings.penColor,
            palette: AnnotationColors.markerPalette,
            // The weight slot is a point size while text is armed, so it has to be
            // fed the size and hand back the size — the row asks one question with
            // one control, and which question it is depends on what is held.
            width: writes ? settings.textSize : settings.strokeWidth,
            widthPresets: writes ? AnnotationMetrics.textPresets : AnnotationMetrics.strokePresets,
            // The ceiling is measured from a page, and a page that has not been
            // laid out yet reports one below the six-point floor — an inverted
            // range is a trap where it is built, not where it is used.
            widthRange: widthRange,
            lineStyle: writes ? nil : settings.style,
            font: writes ? settings.font : nil,
            onFont: { settings.font = $0; onRestyle("font") },
            // Only while a tool that bends is held: a straight caption has no bend
            // to set, and a slot that does nothing is worse than no slot. Gone once
            // the caption has more than one line, because a block does not bend.
            curve: settings.tool.bendsText && settings.textBendApplies
                ? settings.curveDegrees : nil,
            onCurve: { settings.curveDegrees = $0; onRestyle("bend") },
            // Only while a caption is in hand: a turn belongs to the words that
            // were written, not to the ones that will be.
            turn: settings.selectedTextId != nil ? settings.textTurnDegrees : nil,
            onTurn: { settings.textTurnDegrees = $0; onRestyle("turn") },
            onEditing: onEditing,
            onTool: { if let tool = $0 as? AnnotationTool { settings.select(tool) } },
            onColour: { settings.penColor = $0; onRestyle("colour") },
            onWidth: { value in
                if writes { settings.setTextSize(value); onRestyle("size") } else { settings.setStrokeWidth(value) }
            },
            onLineStyle: { settings.style = $0 },
            onPickCustomColour: onPickCustomColour,
            // The reader can put every tool down and go back to plain scrolling. A
            // tool that can only be turned on is a trap, since every touch would
            // keep drawing.
            onDisarm: { settings.select(.none) },
            onDismiss: onDismiss
        )
    }
}

/// Everything a mark-making tool needs, in one row. Android's `MarkRibbon`.
///
/// Colour, weight, line type, then the marks themselves. Read left to right it is
/// one sentence: *this colour, this weight, this line, drawn as this.*
///
/// **Every slot opens on a tap.** These were a long press each, which meant six
/// things hidden behind a gesture with nothing to say they were there. A slot whose
/// glyph shows a group also shows which member is armed, so the row says what is
/// available as well as what is on — somebody who has never opened it can still see
/// what is in there.
///
/// A group of one arms directly instead of opening: a list to choose from a list of
/// one is a step for nothing.
struct MarkRibbon: View {
    let groups: [[RibbonTool]]
    let armed: AnyHashable?
    let colour: MarkColor
    let palette: [MarkColor]
    let width: CGFloat
    let widthPresets: [CGFloat]
    let widthRange: ClosedRange<CGFloat>
    /// Nil when the armed tool has no line type, and then the slot drops out of the
    /// row entirely rather than opening a panel that changes nothing. The
    /// highlighter is the case: a wash has no length to break up.
    let lineStyle: MarkupStyle?
    /// The font, when the armed tool writes words rather than drawing.
    ///
    /// Non-nil swaps two slots: the weight becomes a point size and the line type
    /// becomes the font. Neither a nib width nor a dash means anything to a letter,
    /// and a row that kept them would be offering controls that do nothing.
    var font: PagifyFont?
    var onFont: (PagifyFont) -> Void = { _ in }
    /// How far the baseline turns from end to end, in degrees, or nil when the
    /// armed tool writes on a straight line.
    ///
    /// A slot of its own rather than a mode of the others, because it is a
    /// different question — the size says how big, the font says in what, and this
    /// says along what.
    var curve: CGFloat?
    var onCurve: (CGFloat) -> Void = { _ in }
    /// How far the caption in hand is turned, in degrees. Nil when nothing is in
    /// hand — a turn is about a caption that exists, not a setting for the next
    /// one, which is why this is not on the ribbon unless one is selected.
    var turn: CGFloat?
    var onTurn: (CGFloat) -> Void = { _ in }
    /// A slider is being dragged, or has been let go.
    var onEditing: (Bool) -> Void = { _ in }
    let onTool: (AnyHashable) -> Void
    let onColour: (MarkColor) -> Void
    let onWidth: (CGFloat) -> Void
    let onLineStyle: (MarkupStyle) -> Void
    /// Opens the wheel, for a colour the palette does not carry.
    let onPickCustomColour: () -> Void
    /// The weight slot means "how strong", not "how thick". Same control, same
    /// question — only the range and the way it reads differ.
    var widthIsIntensity: Bool = false
    /// Nil where a tool is always held. The reader can put every tool down and go
    /// back to scrolling; the screenshot editor cannot, because a finger on the
    /// picture there has nothing else to mean.
    var onDisarm: (() -> Void)?
    /// Puts the whole band away. Nil where the band is always on screen and there
    /// is nothing to close.
    var onDismiss: (() -> Void)?

    @State private var open: Panel?
    // Where the slot that opened the panel sits, so the panel opens over it. Global
    // points: the panel and the slot are laid out separately, and the window is the
    // space they share.
    @State private var anchorCentre: CGFloat = 0
    @State private var rowLeft: CGFloat = 0
    @State private var rowWidth: CGFloat = 0
    @State private var panelWidth: CGFloat = 0

    @Environment(\.colorScheme) private var scheme

    /// Which panel the row currently has open, if any.
    private enum Panel: Equatable {
        case colour
        case thickness
        case lineType
        case font
        case curve
        case turn
        case tools([RibbonTool])
    }

    var body: some View {
        VStack(spacing: 8) {
            if let panel = open {
                RibbonPanelSurface(onClose: { open = nil }) { choices(panel) }
                    .background(GeometryReader { geometry in
                        Color.clear.preference(key: PanelWidthKey.self, value: geometry.size.width)
                    })
                    .onPreferenceChange(PanelWidthKey.self) { panelWidth = $0 }
                    .offset(x: shift)
            }
            band
        }
        // A slot that vanishes must not leave its panel behind: the line type drops
        // out of the row the moment the highlighter is armed, the font slot only
        // exists while something that writes words is held, and the bend goes as
        // soon as the caption in hand grows a second line.
        .onChange(of: slotsOnOffer) { _, _ in
            if lineStyle == nil && open == .lineType { open = nil }
            if font == nil && open == .font { open = nil }
            if curve == nil && open == .curve { open = nil }
        }
    }

    private var slotsOnOffer: [Bool] { [lineStyle != nil, font != nil, curve != nil] }

    private var shift: CGFloat {
        guard rowWidth > 0, panelWidth > 0 else { return 0 }
        let limit = max(rowWidth / 2 - panelWidth / 2 - panelEdgeGap, 0)
        return min(max(anchorCentre - (rowLeft + rowWidth / 2), -limit), limit)
    }

    // ------------------------------------------------------------------ row --

    private var band: some View {
        // The cross sits outside the scrolling row, not in it: scrolled along with
        // the slots it spent most of its life off the edge of the screen, which is
        // the one thing a way out cannot be.
        HStack(spacing: 0) {
            // Scrolled, so the slots keep their own size instead of being squeezed
            // by whatever width is left. A plain row given too little measures its
            // last child at nothing: the text slot's glyph went on drawing at full
            // size, over a tap target zero pixels wide, and the tool simply could
            // not be picked. `ViewThatFits` wraps the content while it fits and
            // hands over to a scroller when it does not.
            ViewThatFits(in: .horizontal) {
                slots
                ScrollView(.horizontal) { slots }
                    .scrollIndicators(.hidden)
            }

            if let dismiss = onDismiss {
                RibbonClose {
                    open = nil
                    dismiss()
                }
                .padding(.trailing, 8)
            }
        }
        .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 28))
        .shadow(color: .black.opacity(scheme == .dark ? 0.5 : 0.18), radius: 8, y: 3)
        .background(GeometryReader { geometry in
            Color.clear.preference(key: RowFrameKey.self,
                                   value: RowFrame(left: geometry.frame(in: .global).minX,
                                                   width: geometry.size.width))
        })
        .onPreferenceChange(RowFrameKey.self) { frame in
            rowLeft = frame.left
            rowWidth = frame.width
        }
    }

    private var slots: some View {
        HStack(spacing: 4) {
            RibbonSlot(label: "Colour", onOpen: { toggle(.colour, at: $0) }) {
                ColourGlyph(colour: colour)
            }

            // The same slot asks a different question when what is armed writes
            // words: how big, rather than how thick.
            RibbonSlot(label: font != nil ? "Size" : "Thickness",
                       onOpen: { toggle(.thickness, at: $0) }) {
                if font != nil {
                    SizeGlyph(sizePoints: width)
                } else {
                    ThicknessGlyph(width: width, presets: widthPresets)
                }
            }

            if let curve = curve {
                RibbonSlot(label: "Bend", onOpen: { toggle(.curve, at: $0) }) {
                    CurveGlyph(degrees: curve)
                }
            }

            if let turn = turn {
                RibbonSlot(label: "Turn", onOpen: { toggle(.turn, at: $0) }) {
                    Image(systemName: "rotate.right")
                        .font(.system(size: 17, weight: .medium))
                        .rotationEffect(.degrees(turn))
                }
            }

            // An `else if`, not two conditions. A caption has a face where a stroke
            // has a dash, and the slot in that position asks whichever of the two
            // questions the armed tool can answer.
            if let font = font {
                RibbonSlot(label: "Font", onOpen: { toggle(.font, at: $0) }) {
                    FontGlyph(font: font)
                }
            } else if let lineStyle = lineStyle {
                RibbonSlot(label: "Line type", onOpen: { toggle(.lineType, at: $0) }) {
                    LineTypeGlyph(style: lineStyle)
                }
            }

            ForEach(Array(groups.indices), id: \.self) { index in
                let group = groups[index]
                if group.count == 1, let only = group.first {
                    RibbonSlot(label: only.name, hasMore: false, onOpen: { _ in
                        open = nil
                        if only.key == armed, let disarm = onDisarm {
                            disarm()
                        } else {
                            onTool(only.key)
                        }
                    }) {
                        GroupGlyph(group: group, armed: armed)
                    }
                } else {
                    RibbonSlot(label: group.map(\.name).joined(separator: " or "),
                               onOpen: { centre in
                                   // A tool in this group is in your hand, so the
                                   // slot puts it down. Reaching the armed tool
                                   // inside the picker to disarm meant knowing
                                   // which of them was armed and finding it again;
                                   // the slot is the thing you are already looking
                                   // at. Swapping within the group is the second
                                   // tap.
                                   if group.contains(where: { $0.key == armed }),
                                      let disarm = onDisarm {
                                       open = nil
                                       disarm()
                                   } else {
                                       toggle(.tools(group), at: centre)
                                   }
                               }) {
                        GroupGlyph(group: group, armed: armed)
                    }
                }
            }
        }
        .padding(.leading, 8)
        .padding(.trailing, 4)
        .padding(.vertical, 6)
    }

    private func toggle(_ panel: Panel, at centre: CGFloat) {
        anchorCentre = centre
        open = open == panel ? nil : panel
    }

    // --------------------------------------------------------------- panels --

    @ViewBuilder
    private func choices(_ panel: Panel) -> some View {
        switch panel {
        case .colour:
            ColourChoices(colour: colour, palette: palette,
                          onColour: {
                              onColour($0)
                              open = nil
                          },
                          onWheel: {
                              open = nil
                              onPickCustomColour()
                          })

        case .thickness:
            ThicknessChoices(width: width, colour: colour, presets: widthPresets,
                             range: widthRange, isIntensity: widthIsIntensity,
                             onWidth: onWidth, onEditing: onEditing)

        case .lineType:
            LineTypeChoices(style: lineStyle ?? .solid, onStyle: {
                onLineStyle($0)
                open = nil
            })

        case .font:
            FontChoices(font: font ?? .helvetica, onFont: {
                onFont($0)
                open = nil
            })

        case .curve:
            CurveChoices(degrees: curve ?? 0, onDegrees: onCurve, onEditing: onEditing)

        case .turn:
            TurnChoices(degrees: turn ?? 0, onDegrees: onTurn, onEditing: onEditing)

        case .tools(let tools):
            ToolChoices(tools: tools, armed: armed,
                        onTool: {
                            onTool($0)
                            open = nil
                        },
                        onDisarm: onDisarm.map { disarm in
                            { () -> Void in
                                disarm()
                                open = nil
                            }
                        })
        }
    }
}

// ------------------------------------------------------------------- pieces --

/// One slot: a glyph, a tap target, and the wedge that says it opens something.
///
/// The glyph is passed in rather than named, because half of these are drawn rather
/// than picked from an icon set — a stack of three line weights is not a thing SF
/// Symbols has, and a group's glyph has to show which member is armed.
///
/// No selected ground of any kind: armed-ness is said by the glyph's own tint, and
/// a fill behind it would say it twice in two different colours.
private struct RibbonSlot<Glyph: View>: View {
    let label: String
    var hasMore = true
    let onOpen: (CGFloat) -> Void
    @ViewBuilder let glyph: Glyph

    @State private var centreX: CGFloat = 0

    var body: some View {
        Button { onOpen(centreX) } label: {
            glyph
                .frame(width: slotSize, height: slotSize)
                .contentShape(Circle())
                .overlay(alignment: .bottomTrailing) {
                    // The default inset, not a smaller one: the slot is a circle
                    // before the wedge is drawn, so a wedge tucked further into the
                    // corner comes out as a sliver of itself.
                    if hasMore { MoreTick() }
                }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        .background(GeometryReader { geometry in
            Color.clear.preference(key: SlotCentreKey.self,
                                   value: geometry.frame(in: .global).midX)
        })
        .onPreferenceChange(SlotCentreKey.self) { centreX = $0 }
    }
}

/// The ground every opened panel sits on.
private struct RibbonPanelSurface<Content: View>: View {
    /// Shuts the panel. See the cross below for why it is worth the room.
    let onClose: () -> Void
    @ViewBuilder let content: Content

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 0) {
            content
            // A way out that is visibly a way out. The panel already closes by
            // tapping the slot that opened it or by choosing from it, but neither
            // of those looks like closing — somebody who opened it to see what was
            // there had to pick something to get rid of it.
            RibbonClose(onClick: onClose)
                .padding(.leading, 2)
        }
        .padding(.leading, 10)
        .padding(.trailing, 4)
        .padding(.vertical, 8)
        .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 20))
        .shadow(color: .black.opacity(scheme == .dark ? 0.5 : 0.18), radius: 8, y: 3)
    }
}

/// The band's own way out.
///
/// Filled, where every slot beside it is not. The cross had to sit in the same row
/// as the tools to be anywhere near the thumb, and in that row a bare glyph reads as
/// one more thing to draw with — so it is given a ground of its own, which is the
/// one visual difference nothing else in the row uses.
private struct RibbonClose: View {
    let onClick: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: onClick) {
            Image(systemName: "xmark")
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
                .frame(width: closeSize, height: closeSize)
                .background(Circle().fill(PagifyColor.surfaceVariant(scheme)))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Close")
    }
}

/// The palette, then the way to any other colour.
private struct ColourChoices: View {
    let colour: MarkColor
    let palette: [MarkColor]
    let onColour: (MarkColor) -> Void
    let onWheel: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            // The wheel first: it is the way to *any* colour, so it belongs where
            // the eye starts rather than tucked behind the ones chosen in advance.
            CustomColourSwatch(current: colour, isCustom: !palette.contains(colour),
                               onClick: onWheel, size: 30)
            ForEach(palette, id: \.argb) { swatch in
                ColourDot(colour: swatch, selected: swatch == colour) { onColour(swatch) }
            }
        }
    }
}

/// The presets as taps, and a slider for anything else.
///
/// The presets are what most marks want and a slider alone would make the common
/// case the slow one; the slider is what makes the uncommon case possible at all.
private struct ThicknessChoices: View {
    let width: CGFloat
    let colour: MarkColor
    let presets: [CGFloat]
    let range: ClosedRange<CGFloat>
    let isIntensity: Bool
    let onWidth: (CGFloat) -> Void
    var onEditing: (Bool) -> Void = { _ in }

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        // The same control asks two questions. One ribbon measures a nib in page
        // points; the other measures how strong a wash is, as a fraction of full.
        let readout: String = isIntensity
            ? "\(Int((width * 100).rounded()))%"
            : String(format: "%.1f", width)

        VStack(spacing: 2) {
            HStack(spacing: 10) {
                ForEach(presets, id: \.self) { preset in
                    NibDot(width: preset, heaviest: presets.max() ?? 1, colour: colour,
                           selected: preset == width) { onWidth(preset) }
                }
                Text(readout)
                    .font(.system(size: 14, weight: .medium).monospacedDigit())
                    .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
            }
            // A slider whose bounds have collapsed traps rather than draws, and a
            // text-size ceiling computed from a page that has not measured itself
            // yet can arrive below the six-point floor.
            Slider(value: Binding(get: { min(max(width, safeRange.lowerBound),
                                             safeRange.upperBound) },
                                  set: onWidth),
                   in: safeRange,
                   onEditingChanged: onEditing)
                .frame(width: 220)
        }
    }

    private var safeRange: ClosedRange<CGFloat> {
        range.lowerBound < range.upperBound
            ? range
            : range.lowerBound...(range.lowerBound + 0.001)
    }
}

/// All five line types, drawn in the pattern they name.
private struct LineTypeChoices: View {
    let style: MarkupStyle
    let onStyle: (MarkupStyle) -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 4) {
            ForEach(MarkupStyle.allCases) { option in
                Button { onStyle(option) } label: {
                    LinePattern(style: option,
                                // Ink on the amber, not the theme's on-primary:
                                // that is a pale colour meant for the theme's own
                                // accent, and on this one it disappears.
                                tint: option == style
                                    ? PagifyColor.accentInk
                                    : PagifyColor.onSurfaceVariant(scheme),
                                width: 40)
                        .frame(width: 52, height: 40)
                        .background(RoundedRectangle(cornerRadius: 12)
                            .fill(option == style ? PagifyColor.ribbonAccent : .clear))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(option.label)
            }
        }
    }
}

/// The faces, each written in itself.
///
/// A list of names in one font tells you nothing about any of them. What is on
/// screen for a standard-14 is only a likeness — the phone does not have Helvetica —
/// but the difference between a serif and a sans is exactly what the choice is
/// about, and that much a likeness carries.
private struct FontChoices: View {
    let font: PagifyFont
    let onFont: (PagifyFont) -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        // Scrolls: there are twenty-odd faces, and a column of them is taller than
        // the phone. A non-scrolling row of them is how most of the list became
        // physically unreachable.
        ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(PagifyFont.allCases) { option in
                    let live = option == font
                    Button { onFont(option) } label: {
                        VStack(alignment: .leading, spacing: 0) {
                            // The face itself where there is one. Without it a name
                            // written in Devanagari or Korean is drawn by whatever
                            // the phone falls back to, which is the one thing the
                            // label was meant to show.
                            Text(option.label)
                                .font(RibbonFontFaces.specimen(option, size: 17))
                                .foregroundStyle(live ? PagifyColor.accentInk
                                                 : PagifyColor.onSurface(scheme))
                            Text(option.script)
                                .font(.system(size: 11))
                                .foregroundStyle(live
                                                 ? PagifyColor.accentInk.opacity(0.75)
                                                 : PagifyColor.onSurfaceVariant(scheme))
                        }
                        .padding(.horizontal, 14)
                        .padding(.vertical, 8)
                        .background(RoundedRectangle(cornerRadius: 10)
                            .fill(live ? PagifyColor.ribbonAccent : .clear))
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .frame(maxHeight: fontListHeight)
        .fixedSize(horizontal: true, vertical: false)
    }
}

/// How far the line bends, as a row of shapes and a slider.
///
/// The presets are the three answers most captions want — sagging, straight,
/// arching — and the slider is there for the one that wants something else. The
/// panel stays open behind them: bending a caption is something you do by watching
/// it, not by choosing once.
private struct CurveChoices: View {
    let degrees: CGFloat
    let onDegrees: (CGFloat) -> Void
    /// Whether the slider is being dragged, so the write can be held back until
    /// it is let go. See `ReaderModel.beginRestyle`.
    var onEditing: (Bool) -> Void = { _ in }

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(spacing: 2) {
            HStack(spacing: 10) {
                ForEach(AnnotationMetrics.curvePresets, id: \.self) { preset in
                    Button { onDegrees(preset) } label: {
                        CurveGlyph(degrees: preset, lit: preset == degrees, size: 26)
                            .frame(width: 38, height: 38)
                            .contentShape(Circle())
                    }
                    .buttonStyle(.plain)
                }
                Text("\(Int(degrees.rounded()))°" as String)
                    .font(.system(size: 14, weight: .medium).monospacedDigit())
                    .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
            }
            Slider(value: Binding(get: { degrees }, set: onDegrees),
                   in: -AnnotationMetrics.curveLimit...AnnotationMetrics.curveLimit,
                   onEditingChanged: onEditing)
                .frame(width: 220)
        }
    }
}

/// How far a caption is turned on the page.
///
/// The quarter turns are presets because they are what anyone actually wants —
/// words up the side of a page, or upside down against a plan — and hitting 90°
/// exactly by dragging a slider is a game. The slider is there for the angles in
/// between, which are the ones a drawing needs.
private struct TurnChoices: View {
    let degrees: CGFloat
    let onDegrees: (CGFloat) -> Void
    var onEditing: (Bool) -> Void = { _ in }

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(spacing: 2) {
            HStack(spacing: 10) {
                ForEach([CGFloat(0), 90, 180, 270], id: \.self) { preset in
                    Button { onDegrees(preset) } label: {
                        Image(systemName: "textformat")
                            .font(.system(size: 15, weight: .medium))
                            .rotationEffect(.degrees(preset))
                            .frame(width: 38, height: 38)
                            .background(abs(preset - degrees) < 0.5
                                        ? PagifyColor.primary(scheme).opacity(0.18)
                                        : .clear, in: Circle())
                            .contentShape(Circle())
                    }
                    .buttonStyle(.plain)
                }
                Text("\(Int(degrees.rounded()))°" as String)
                    .font(.system(size: 14, weight: .medium).monospacedDigit())
                    .foregroundStyle(PagifyColor.onSurfaceVariant(scheme))
            }
            Slider(value: Binding(get: { degrees }, set: onDegrees), in: 0...359,
                   onEditingChanged: onEditing)
                .frame(width: 220)
        }
    }
}

/// The members of a group, to pick one from.
private struct ToolChoices: View {
    let tools: [RibbonTool]
    let armed: AnyHashable?
    let onTool: (AnyHashable) -> Void
    let onDisarm: (() -> Void)?

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(spacing: 2) {
            ForEach(tools) { tool in
                let live = tool.key == armed
                Button {
                    // Tapping the armed one puts it down, where putting it down is
                    // a thing that can be done at all.
                    if live, let disarm = onDisarm { disarm() } else { onTool(tool.key) }
                } label: {
                    RibbonGlyphView(icon: tool.icon, size: 24)
                        .foregroundStyle(live ? PagifyColor.accentInk
                                         : PagifyColor.onSurfaceVariant(scheme))
                        .frame(width: slotSize, height: slotSize)
                        .background(Circle().fill(live ? PagifyColor.ribbonAccent : .clear))
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(tool.name)
            }
        }
    }
}

// ------------------------------------------------------------- the toolbar --

/// A tool in the reader's bottom row.
///
/// Tap and long press both, because that is the whole gesture vocabulary of this
/// row: a tap chooses, a press adjusts. Attached as two gestures rather than as a
/// `Button` with a press bolted on, so the tap loses to the press instead of both
/// firing when a finger rests.
private struct ToolButton: View {
    @State private var didLongPress = false
    let icon: RibbonIcon
    let label: String
    var selected = false
    /// A dot of the current ink, so the pen's colour is visible without opening the
    /// palette. Only the two slots that draw carry one.
    var accent: MarkColor?
    /// Draws the wedge that says a long press offers more.
    var hasMore = false
    let onClick: () -> Void
    var onLongPress: (() -> Void)?

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Group {
            // Attached only where there is something behind the press. A long press
            // wired to nothing still beats the tap to the gesture, so a finger that
            // rested on the Note slot would arm nothing at all.
            if let onLongPress = onLongPress {
                face.onLongPressGesture(minimumDuration: 0.45, perform: onLongPress)
            } else {
                face
            }
        }
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(label)
    }

    private var face: some View {
        RibbonGlyphView(icon: icon, size: 22)
            .foregroundStyle(selected ? PagifyColor.onPrimary(scheme)
                             : PagifyColor.onSurfaceVariant(scheme))
            .frame(width: slotSize, height: slotSize)
            .background(Circle().fill(selected ? PagifyColor.primary(scheme) : .clear))
            .overlay(alignment: .bottom) {
                if let accent = accent {
                    Circle()
                        .fill(Color(accent.cgColor))
                        .frame(width: 6, height: 6)
                        .padding(.bottom, 6)
                }
            }
            .overlay(alignment: .bottomTrailing) {
                if hasMore { MoreTick() }
            }
            .contentShape(Circle())
            .onTapGesture {
                    guard !didLongPress else { didLongPress = false; return }
                    onClick()
                }
    }
}

/// What the armed tool draws with, as a band above the row.
///
/// Only the colour half is here, and that is not an omission: the band is only ever
/// shown for the highlighter, which has a colour and nothing else. Showing it a nib
/// width and a line type — controls that do nothing to a wash — would be
/// three-quarters of a band pretending to work. Every drawing tool carries its own
/// settings in the drawing ribbon instead.
private struct MarkParameters: View {
    let tool: AnnotationTool
    let colour: MarkColor
    let onColour: (MarkColor) -> Void
    /// Opens the wheel, for a colour none of the six offers.
    let onPickCustomColour: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        // The highlighter's washes have to read behind text; ink has to read on
        // white. Same row, different set.
        let palette = tool == .highlight
            ? AnnotationColors.highlightPalette
            : AnnotationColors.markerPalette

        let row = HStack(spacing: 5) {
            // The wheel first, before the six, exactly as in the drawing ribbon.
            // Offered for the wash too: six pale colours are what a highlighter
            // usually wants, but "usually" is not "only" — a document already marked
            // up in a house colour needs that colour, and deciding on someone else's
            // behalf that they could not want it was not ours to make.
            CustomColourSwatch(current: colour, isCustom: !palette.contains(colour),
                               onClick: onPickCustomColour, size: 30)
            ForEach(palette, id: \.argb) { swatch in
                ColourDot(colour: swatch, selected: swatch == colour) { onColour(swatch) }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)

        // As wide as the swatches need, and no wider, until they stop fitting.
        ViewThatFits(in: .horizontal) {
            row
            ScrollView(.horizontal) { row }
                .scrollIndicators(.hidden)
        }
        .frame(maxWidth: parameterBandWidth)
        .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 20))
        .shadow(color: .black.opacity(scheme == .dark ? 0.5 : 0.18), radius: 6, y: 3)
    }
}

/// The wider erasures, behind a long press on the eraser.
///
/// Both confirm before they run. Undo would bring the marks back either way, but a
/// wipe you did not mean to trigger is alarming in a way a single erased highlight
/// is not, and the count in the prompt is what tells you which of the two actions
/// you are about to take.
private struct ClearMenu: View {
    let marksOnPage: Int
    let marksInDocument: Int
    let onClearPage: () -> Void
    let onClearAll: () -> Void
    let onDismiss: () -> Void

    @State private var confirming: ClearScope?
    @Environment(\.colorScheme) private var scheme

    private enum ClearScope { case page, document }

    var body: some View {
        HStack(spacing: 10) {
            ClearChip(label: "Clear page (\(marksOnPage))", enabled: marksOnPage > 0) {
                confirming = .page
            }
            ClearChip(label: "Clear all (\(marksInDocument))", enabled: marksInDocument > 0) {
                confirming = .document
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(PagifyColor.surface(scheme), in: RoundedRectangle(cornerRadius: 20))
        .shadow(color: .black.opacity(scheme == .dark ? 0.5 : 0.18), radius: 8, y: 3)
        .alert(confirmTitle,
               isPresented: Binding(get: { confirming != nil },
                                    set: { if !$0 { confirming = nil } })) {
            Button("Clear", role: .destructive) {
                if confirming == .page { onClearPage() } else { onClearAll() }
                confirming = nil
                onDismiss()
            }
            Button("Cancel", role: .cancel) { confirming = nil }
        } message: {
            let count = confirming == .page ? marksOnPage : marksInDocument
            let scope = confirming == .page ? "this page" : "the whole document"
            let marks = count == 1 ? "1 mark" : "\(count) marks"
            Text("This removes \(marks) from \(scope). You can undo it." as String)
        }
    }

    private var confirmTitle: String {
        confirming == .page ? "Clear this page?" : "Clear everything?"
    }
}

private struct ClearChip: View {
    let label: String
    let enabled: Bool
    let onClick: () -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Button(action: onClick) {
            Text(label)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(enabled ? PagifyColor.onErrorContainer(scheme) : PagifyColor.onSurfaceVariant(scheme).opacity(0.5))
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 14)
                    .fill(enabled ? PagifyColor.errorContainer(scheme) : PagifyColor.surfaceVariant(scheme)))
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }
}

// ---------------------------------------------------------------- geometry --

/// How wide a slot is. The same in both ribbons, so the two rows line up.
private let slotSize: CGFloat = 46

/// Big enough to hit, small enough not to look like one of the choices.
private let closeSize: CGFloat = 30

/// As tall as the font list may get before it starts scrolling instead.
private let fontListHeight: CGFloat = 420

/// How close a shifted panel may come to the edge of the row.
private let panelEdgeGap: CGFloat = 8

/// How wide the parameters band may grow before its colours start scrolling.
private let parameterBandWidth: CGFloat = 360

private struct SlotCentreKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

private struct RowFrame: Equatable {
    var left: CGFloat = 0
    var width: CGFloat = 0
}

private struct RowFrameKey: PreferenceKey {
    static let defaultValue = RowFrame()
    static func reduce(value: inout RowFrame, nextValue: () -> RowFrame) { value = nextValue() }
}

private struct PanelWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}
