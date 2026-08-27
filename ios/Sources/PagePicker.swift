import SwiftUI

/// Choosing pages out of a document.
///
/// Takes a document rather than the reader's model, because it is used against
/// two of them: the one being read, when choosing what to export, and a file
/// just opened, when choosing what to import from it. Neither knows about the
/// other, and only one of them is the document the reader is editing.
///
/// Selection is a *list*, not a set. The order pages are tapped in is the order
/// they come out in — "give me page 3, then page 1" is a thing somebody can ask
/// for, and quietly sorting it hands them a different document. `exportPages`
/// and `importPages` both take the list as given, so the position each chosen
/// page shows is what will actually happen.
struct PagePicker: View {
    let document: PagifyDocument

    /// Changes when the pages stop being what is drawn, so the thumbnails are
    /// re-rendered. Left at zero for a file opened only to import from: nothing
    /// here can edit it.
    var revision: Int = 0

    /// What the button says, e.g. "Export 3 pages".
    let confirmLabel: (Int) -> String
    let onConfirm: ([Int]) -> Void
    let onCancel: () -> Void

    @Environment(\.colorScheme) private var scheme
    @State private var chosen: [Int] = []

    private var pageCount: Int { document.pageCount }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            grid
            footer
        }
        .padding(.horizontal, 16)
        .background(PagifyColor.background(scheme))
    }

    private var header: some View {
        HStack {
            Text(chosen.isEmpty ? "Tap the pages you want"
                                : "\(chosen.count) of \(pageCount) chosen")
                .font(.headline)

            Spacer()

            Button(chosen.count == pageCount ? "None" : "All") {
                chosen = chosen.count == pageCount ? [] : Array(0..<pageCount)
            }
        }
        .padding(.top, 16)
    }

    private var grid: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 108), spacing: 10)], spacing: 10) {
                ForEach(Array(0..<pageCount), id: \.self) { index in
                    let position = chosen.firstIndex(of: index).map { $0 + 1 }
                    // The rail's cell, which already draws a page at its own
                    // shape and rings the one that is picked out. What "picked
                    // out" means is the only thing that differs here, and it is
                    // the badge that says so.
                    PageThumbnail(document: document,
                                  index: index,
                                  revision: revision,
                                  isCurrent: position != nil,
                                  onSelect: { toggle(index) })
                        .overlay(alignment: .topTrailing) {
                            if let position { badge(position) }
                        }
                        // Replacing the rail's own label, which says "Go to
                        // page" — nothing here goes anywhere, and the position
                        // is the part a badge cannot read out.
                        .accessibilityLabel(position.map {
                            "Page \(index + 1), chosen at position \($0)"
                        } ?? "Page \(index + 1)")
                }
            }
            .padding(.vertical, 12)
        }
    }

    private var footer: some View {
        HStack {
            Spacer()
            Button("Cancel", action: onCancel)
            Button(confirmLabel(chosen.count)) { onConfirm(chosen) }
                .buttonStyle(.borderedProminent)
                .disabled(chosen.isEmpty)
        }
        .padding(.bottom, 16)
    }

    /// The position in the chosen list, not a tick.
    ///
    /// With order mattering, "3rd" is the fact somebody needs to see, and a tick
    /// would hide it. It never takes the tap: the whole cell is the target, and
    /// a badge that swallowed touches would make a chosen page impossible to
    /// unchoose.
    private func badge(_ position: Int) -> some View {
        ZStack {
            Circle().fill(PagifyColor.primary(scheme))
            if position > Self.mostShownAsANumber {
                Image(systemName: "checkmark")
                    .font(.system(size: 11, weight: .bold))
            } else {
                Text("\(position)").font(.caption2.weight(.semibold))
            }
        }
        .foregroundStyle(PagifyColor.onPrimary(scheme))
        .frame(width: 22, height: 22)
        .padding(4)
        .allowsHitTesting(false)
    }

    private func toggle(_ index: Int) {
        if let at = chosen.firstIndex(of: index) {
            chosen.remove(at: at)
        } else {
            chosen.append(index)
        }
    }

    /// Past this, the badge shows a tick instead of a number.
    ///
    /// Two digits fit; three do not, and a "123" squeezed to illegibility is
    /// worse than a tick. Somebody selecting a hundred pages is selecting all of
    /// them anyway.
    private static let mostShownAsANumber = 99
}
