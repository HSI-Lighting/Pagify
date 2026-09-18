import SwiftUI

/// One editable line in the review sheet — the panel's-worth of a single
/// `Field`/`PhoneField`, flattened out of `BusinessCard` and numbered for the
/// badges drawn on the photo above it.
private struct ReviewableField: Identifiable {
    enum Kind {
        case name, title, company, address, notes
        case phone(PhoneKind)
        case email, url
    }

    let id = UUID()
    let kind: Kind
    var value: String
    /// `nil` for anything with no photograph to point at — a QR-sourced
    /// value, or `notes` (a plain `String` in `BusinessCard`, never a
    /// `Field`). Such a row gets no badge and no number.
    let region: Region?
    var removed = false
    /// Only meaningful for `.phone` — the machine-normalised value this row
    /// started the sheet with, kept alongside rather than reconstructed from
    /// `value` later, since two phones can share the same raw text.
    var originalNormalised: String = ""

    var label: String {
        switch kind {
        case .name: return "Name"
        case .title: return "Title"
        case .company: return "Company"
        case .address: return "Address"
        case .notes: return "Notes"
        case .phone(let phoneKind):
            switch phoneKind {
            case .cell: return "Mobile"
            case .work: return "Work"
            case .home: return "Home"
            case .fax: return "Fax"
            }
        case .email: return "Email"
        case .url: return "Website"
        }
    }
}

/// §B.5's review step. Only reached when at least one field has a region to
/// show — a card read entirely from a QR code has nowhere on a photograph to
/// point at, and is saved straight through by whoever presents this sheet.
///
/// **Values are listed in a numbered panel below the photo, never drawn in
/// place on it.** The first attempt at this drew each value directly over its
/// region on a dimmed photo and failed in practice: a business card's lines
/// sit a few millimetres apart and readable type does not, so labelled values
/// overlapped both each other and the card's own printed text. Only a small
/// numbered badge is drawn on the photo — matched to its row by number — and
/// even that moved once already: it originally sat on the left edge of its
/// region and covered the value's first letter ("Yaseen Anwar" read as
/// "aseen Anwar"), and now sits just clear of it.
struct CardReviewSheet: View {
    let image: UIImage
    let card: BusinessCard
    /// Matches Android's own `textScale`: a multiplier from the "Card review
    /// text" setting, since this panel is read at arm's length, often in
    /// poor light, by somebody holding the card in their other hand.
    let textScale: CGFloat
    let onSave: (BusinessCard) -> Void
    let onCancel: () -> Void

    @State private var fields: [ReviewableField]
    @Environment(\.dismiss) private var dismiss

    /// **Show the card, not the desk it was lying on.** A photograph is
    /// framed for a camera, so the card is often a small rectangle
    /// surrounded by floor — fitting the whole frame made the one thing
    /// being checked the smallest thing on the screen, and left the panel
    /// below fighting the keyboard for whatever height the full photo
    /// didn't already claim.
    ///
    /// The crop is the extent of every field's region plus a margin — the
    /// same measurement the Rust parser's own `around_text` uses to size a
    /// card in the first place, matching Android's `cropAround` exactly,
    /// including its fallback: with no regions at all (nothing to crop to)
    /// this is the whole frame, which is then the only honest thing to show.
    private let cropRect: CGRect
    private let croppedImage: UIImage

    /// Blank card stock the words never reach, as a fraction of the text's
    /// own extent — same constant Android tunes this against.
    private static let cropMargin: CGFloat = 0.12
    /// The panel never shrinks below this, however tall the cropped card is.
    private static let panelMinHeight: CGFloat = 200

    init(image: UIImage, card: BusinessCard, textScale: CGFloat, onSave: @escaping (BusinessCard) -> Void, onCancel: @escaping () -> Void) {
        self.image = image
        self.card = card
        self.textScale = textScale
        self.onSave = onSave
        self.onCancel = onCancel
        let fields = Self.flatten(card)
        _fields = State(initialValue: fields)
        let crop = Self.cropRect(around: fields, imageSize: image.size, margin: Self.cropMargin)
        self.cropRect = crop
        self.croppedImage = Self.cut(image, to: crop) ?? image
    }

    /// Matches Android's `cropAround`: an unconditional min/max envelope
    /// over every field's region, padded proportionally to the text's own
    /// size so it holds at any resolution. Not protected against a stray
    /// region ballooning it — the same known, deliberately unpatched gap
    /// `around_text` and `cropAround` both carry, for the same reason: no
    /// distance threshold is justified without a captured card showing what
    /// a real stray fragment looks like.
    private static func cropRect(around fields: [ReviewableField], imageSize: CGSize, margin: CGFloat) -> CGRect {
        let whole = CGRect(origin: .zero, size: imageSize)
        let regions = fields.compactMap(\.region)
        guard !regions.isEmpty else { return whole }

        let left = regions.map(\.left).min()!
        let top = regions.map(\.top).min()!
        let right = regions.map(\.right).max()!
        let bottom = regions.map(\.bottom).max()!

        let padX = (right - left) * margin
        let padY = (bottom - top) * margin
        let rect = CGRect(
            x: max(0, left - padX),
            y: max(0, top - padY),
            width: min(imageSize.width, right + padX) - max(0, left - padX),
            height: min(imageSize.height, bottom + padY) - max(0, top - padY)
        )
        return (rect.width > 1 && rect.height > 1) ? rect : whole
    }

    /// The bitmap is actually cut, rather than drawn oversized and slid
    /// under a clip — matches Android's own comment on why: geometry
    /// mistakes are what this whole screen exists to catch, and a clipped-
    /// but-still-full-size image can look right while being measured wrong.
    private static func cut(_ image: UIImage, to rect: CGRect) -> UIImage? {
        guard let cgImage = image.cgImage else { return nil }
        let scaleX = CGFloat(cgImage.width) / image.size.width
        let scaleY = CGFloat(cgImage.height) / image.size.height
        let pixelRect = CGRect(
            x: rect.minX * scaleX, y: rect.minY * scaleY,
            width: rect.width * scaleX, height: rect.height * scaleY
        ).integral
        guard let cut = cgImage.cropping(to: pixelRect) else { return nil }
        return UIImage(cgImage: cut, scale: 1, orientation: .up)
    }

    private static func flatten(_ card: BusinessCard) -> [ReviewableField] {
        var fields: [ReviewableField] = []
        if let name = card.name { fields.append(.init(kind: .name, value: name.value, region: name.region)) }
        if let title = card.title { fields.append(.init(kind: .title, value: title.value, region: title.region)) }
        if let company = card.company { fields.append(.init(kind: .company, value: company.value, region: company.region)) }
        for phone in card.phones {
            fields.append(.init(
                kind: .phone(phone.kind), value: phone.raw, region: phone.region,
                originalNormalised: phone.normalised
            ))
        }
        for email in card.emails { fields.append(.init(kind: .email, value: email.value, region: email.region)) }
        for url in card.urls { fields.append(.init(kind: .url, value: url.value, region: url.region)) }
        if let address = card.address { fields.append(.init(kind: .address, value: address.value, region: address.region)) }
        if let notes = card.notes, !notes.isEmpty { fields.append(.init(kind: .notes, value: notes, region: nil)) }
        return fields
    }

    /// Numbers only the fields with somewhere on the photo to point at — a
    /// row with no region gets no badge and is skipped when counting, so the
    /// numbers on the photo and in the panel always agree.
    private var numbered: [(number: Int, index: Int)] {
        var result: [(Int, Int)] = []
        var next = 1
        for (index, field) in fields.enumerated() where field.region != nil && !field.removed {
            result.append((next, index))
            next += 1
        }
        return result
    }

    var body: some View {
        NavigationStack {
            GeometryReader { outer in
                // A landscape card needs little height once cropped, and
                // what it doesn't need goes to the panel rather than to
                // empty margin — matches Android's `forPhoto`/`PANEL_MIN`
                // split exactly, computed against this screen's actual
                // size rather than a fixed guess.
                let widthLimited = cropRect.height * (outer.size.width / cropRect.width)
                let forPhoto = min(widthLimited, outer.size.height - Self.panelMinHeight)

                VStack(spacing: 0) {
                    GeometryReader { geometry in
                        let scale = geometry.size.width / cropRect.width
                        ZStack(alignment: .topLeading) {
                            Image(uiImage: croppedImage)
                                .resizable()
                                .aspectRatio(contentMode: .fit)
                                .frame(width: geometry.size.width)
                            ForEach(numbered, id: \.index) { entry in
                                if let region = fields[entry.index].region {
                                    // Relative to the crop, not the full
                                    // photo — the crop is what is drawn.
                                    let left = (CGFloat(region.left) - cropRect.minX) * scale
                                    let top = (CGFloat(region.top) - cropRect.minY) * scale
                                    let width = (CGFloat(region.right) - CGFloat(region.left)) * scale
                                    let height = (CGFloat(region.bottom) - CGFloat(region.top)) * scale

                                    // Which detail was taken FROM the photo,
                                    // not just which one a number points at —
                                    // matches Android's own highlight
                                    // (`.border` + a 22%-alpha `.background`
                                    // over the region), drawn under the badge
                                    // rather than instead of it.
                                    highlightBox(width: width, height: height)
                                        .position(x: left + width / 2, y: top + height / 2)

                                    badge(entry.number)
                                        // Just clear of the region's leading
                                        // edge, not on top of it — see the
                                        // type's own doc comment for why
                                        // that matters.
                                        .position(
                                            x: left - 12,
                                            y: (CGFloat((region.top + region.bottom) / 2) - cropRect.minY) * scale
                                        )
                                }
                            }
                        }
                    }
                    .frame(height: max(forPhoto, 0))
                    // Matches Android's own `clipToBounds()` on this same
                    // marker layer: a stray region from a `split_cards`
                    // misattribution (a known, reachable failure) should be
                    // cut off at the photo's edge rather than spill onto the
                    // panel below and read as the transform having come
                    // loose. Containment, not a threshold.
                    .clipped()

                    List {
                    Section {
                        ForEach($fields) { $field in
                            if !field.removed {
                                fieldRow($field, number: numbered.first { fields[$0.index].id == field.id }?.number)
                            }
                        }
                    }

                    let removedFields = fields.filter(\.removed)
                    if !removedFields.isEmpty {
                        Section("Removed") {
                            ForEach(removedFields) { field in
                                Button {
                                    restoreField(field.id)
                                } label: {
                                    Label(field.value.isEmpty ? field.label : field.value, systemImage: "arrow.uturn.backward")
                                }
                            }
                        }
                    }
                }
                .listStyle(.plain)
            }
            .navigationTitle("Review Card")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { onCancel(); dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { onSave(rebuiltCard()); dismiss() }
                }
            }
            }
        }
    }

    private func badge(_ number: Int) -> some View {
        Text("\(number)")
            .font(.system(size: 12 * textScale, weight: .bold))
            .foregroundStyle(.white)
            .frame(width: 20 * textScale, height: 20 * textScale)
            .background(Circle().fill(.blue))
            .shadow(radius: 2)
    }

    /// `MaterialTheme.colorScheme.primary` on Android — `.blue` is what the
    /// numbered badge already stands in for it with, so the highlight uses
    /// the same colour rather than introducing a second accent.
    private func highlightBox(width: CGFloat, height: CGFloat) -> some View {
        RoundedRectangle(cornerRadius: 3)
            .fill(Color.blue.opacity(0.22))
            .overlay(RoundedRectangle(cornerRadius: 3).stroke(.blue, lineWidth: 2))
            .frame(width: max(width, 0), height: max(height, 0))
    }

    @ViewBuilder
    private func fieldRow(_ field: Binding<ReviewableField>, number: Int?) -> some View {
        HStack(alignment: .top) {
            if let number {
                Text("\(number)")
                    .font(.system(size: 12 * textScale, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 18 * textScale, height: 18 * textScale)
                    .background(Circle().fill(.blue))
                    .padding(.top, 4)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(field.wrappedValue.label)
                    .font(.system(size: 13 * textScale))
                    .foregroundStyle(.secondary)
                TextField(field.wrappedValue.label, text: field.value)
                    .font(.system(size: 17 * textScale))
                    // A hand-corrected phone number no longer matches what
                    // was originally parsed — exporting it inconsistent with
                    // its own displayed text would be worse than exporting
                    // neither, so editing drops the machine-normalised value
                    // and lets the writer fall back to the printed text
                    // (`to_vcard` already does this whenever `normalised` is
                    // empty).
                    .onChange(of: field.wrappedValue.value) { _, _ in
                        if case .phone = field.wrappedValue.kind {
                            phoneNormalisedOverrides[field.id] = ""
                        }
                    }
            }
        }
        .swipeActions(edge: .trailing) {
            Button(role: .destructive) {
                field.wrappedValue.removed = true
            } label: {
                Label("Remove", systemImage: "trash")
            }
        }
    }

    /// Keyed by `ReviewableField.id` rather than folded into the struct
    /// itself: only a phone field a person actually edited should lose its
    /// normalised value, and this stays out of the way of every field that
    /// was never touched.
    @State private var phoneNormalisedOverrides: [UUID: String] = [:]

    private func restoreField(_ id: UUID) {
        guard let index = fields.firstIndex(where: { $0.id == id }) else { return }
        fields[index].removed = false
    }

    private func rebuiltCard() -> BusinessCard {
        var result = card
        result.phones = []
        result.emails = []
        result.urls = []

        func field(_ value: String, region: Region?) -> Field? {
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : Field(value: trimmed, confidence: 1, region: region)
        }

        for entry in fields where !entry.removed {
            switch entry.kind {
            case .name: result.name = field(entry.value, region: entry.region)
            case .title: result.title = field(entry.value, region: entry.region)
            case .company: result.company = field(entry.value, region: entry.region)
            case .address: result.address = field(entry.value, region: entry.region)
            case .notes:
                let trimmed = entry.value.trimmingCharacters(in: .whitespacesAndNewlines)
                result.notes = trimmed.isEmpty ? nil : trimmed
            case .phone(let kind):
                let normalised = phoneNormalisedOverrides[entry.id] ?? entry.originalNormalised
                result.phones.append(PhoneField(
                    raw: entry.value, normalised: normalised, kind: kind, confidence: 1, region: entry.region
                ))
            case .email:
                if let f = field(entry.value, region: entry.region) { result.emails.append(f) }
            case .url:
                if let f = field(entry.value, region: entry.region) { result.urls.append(f) }
            }
        }
        return result
    }
}
