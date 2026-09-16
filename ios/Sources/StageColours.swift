import SwiftUI

/// One colour per stage, in pipeline order — grey, blue, amber, violet,
/// green/red — so the ordering itself reads as progress without anyone
/// having to learn the scheme. Uses SwiftUI's own dynamic `Color`s rather
/// than Android's hand-picked per-theme luminance pairs: a deliberate
/// simplification, not a different design — the same property (readable,
/// sufficient contrast in both themes) holds, delivered by the platform
/// instead of a lookup table.
extension DealStage {
    var colour: Color {
        switch self {
        case .new: return .gray
        case .contacted: return .blue
        case .meeting: return .orange
        case .quoted: return .purple
        case .won: return .green
        case .lost: return .red
        }
    }

    var label: String {
        switch self {
        case .new: return "New"
        case .contacted: return "Contacted"
        case .meeting: return "Meeting"
        case .quoted: return "Quoted"
        case .won: return "Won"
        case .lost: return "Lost"
        }
    }
}

/// Shown for every stage, `.new` included — hiding it there would make
/// "nothing has happened yet" indistinguishable from a row that simply
/// hasn't finished loading.
struct StageBadge: View {
    let stage: DealStage

    var body: some View {
        Text(stage.label)
            .font(.caption2.bold())
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(stage.colour.opacity(0.18), in: Capsule())
            .foregroundStyle(stage.colour)
    }
}
