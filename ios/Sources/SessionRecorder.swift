import Foundation
#if canImport(UIKit)
import UIKit
#endif

/// Records what the renderer actually did during a session, to a plain text file.
///
/// The point is to replace guesswork with a timeline. Every thumbnail and every
/// page reports when it was asked for, when its outline was laid out, and when
/// each pass of pixels arrived — so a scroll that felt slow can be read back as
/// "these eleven thumbnails each took 240 ms and three were cache misses" rather
/// than described from memory.
///
/// Written as text on purpose: the file is meant to be pulled off the device and
/// read, by a person or by a model, without any tooling. Not JSON, and with no
/// viewer — the moment reading it needs a tool it stops being reached for.
///
/// Call sites are cheap when not recording — one flag read and an early return —
/// so the hooks stay in release builds. A recorder that has to be compiled in is
/// a recorder that is never on when the bug happens.
///
/// Nothing is uploaded and nothing leaves the device. `detail` carries page
/// indices, pixel sizes, durations and counts — never page text, never annotation
/// text, never a path outside the app's own directory.
final class SessionRecorder: @unchecked Sendable {
    static let shared = SessionRecorder()

    /// One recorded moment. Fields stay untyped; the file is for reading.
    private struct Event {
        let atMillis: Int
        let kind: String
        let detail: String
        let durationMillis: Int?
    }

    private let lock = NSLock()
    private var events: [Event] = []
    private var startedAtNanos: UInt64 = 0
    private var header = ""

    /// Read without the lock on purpose. This is the idle fast path, hit from
    /// render callbacks and gesture handlers at frame rate, and it must not
    /// become a contention point. The cost of the race is one event either side
    /// of a start or stop.
    private var recording = false

    var isRecording: Bool { recording }

    private init() {}

    func start(documentName: String, pageCount: Int, engineVersion: String) {
        lock.lock()
        events.removeAll()
        // Monotonic. `Date()` can step backwards, which puts negative gaps in a
        // timeline whose whole value is the gaps.
        startedAtNanos = DispatchTime.now().uptimeNanoseconds
        header = """
        Pagify session recording
        document : \(documentName)
        pages    : \(pageCount)
        device   : \(Self.deviceNote)
        engine   : pdf_core \(engineVersion)

        t(ms)   event            detail
        ------  ---------------  \(String(repeating: "-", count: 60))

        """
        lock.unlock()
        recording = true
    }

    /// - Parameter durationMillis: how long the work took, when the event marks a
    ///   completion. Left nil for instantaneous marks like an outline appearing.
    /// The detail is built **only if** a recording is running.
    ///
    /// Taken as an autoclosure because several call sites fire at frame rate and
    /// pass a `String(format:)`. As a plain parameter that string was formatted
    /// and allocated on every call, recording or not — paid for permanently to be
    /// thrown away.
    func record(_ kind: String, _ detail: @autoclosure () -> String, durationMillis: Int? = nil) {
        guard recording else { return }
        let at = Int((DispatchTime.now().uptimeNanoseconds &- startedAtNanos) / 1_000_000)

        lock.lock()
        events.append(Event(atMillis: at, kind: kind, detail: detail(),
                            durationMillis: durationMillis))
        lock.unlock()
    }

    /// Time a piece of work and record it with its duration.
    func timing<T>(_ kind: String, _ detail: String, _ body: () throws -> T) rethrows -> T {
        guard recording else { return try body() }
        let began = DispatchTime.now().uptimeNanoseconds
        let result = try body()
        record(kind, detail,
               durationMillis: Int((DispatchTime.now().uptimeNanoseconds &- began) / 1_000_000))
        return result
    }

    /// Stop recording and write the timeline.
    ///
    /// - Returns: the file written, or nil if nothing was being recorded.
    @discardableResult
    func stop(directory: URL) -> URL? {
        guard recording else { return nil }
        recording = false

        lock.lock()
        let snapshot = events
        events.removeAll()
        let heading = header
        lock.unlock()

        var text = heading
        for event in snapshot {
            text += String(event.atMillis).leftPadded(to: 6)
            text += "  "
            text += event.kind.rightPadded(to: 15)
            text += "  "
            text += event.detail
            if let took = event.durationMillis { text += "  took=\(took)ms" }
            text += "\n"
        }
        text += "\n"
        text += Self.summarise(snapshot)

        let file = directory.appendingPathComponent(
            "pagify-session-\(Int(Date().timeIntervalSince1970 * 1000)).txt")
        guard (try? text.write(to: file, atomically: true, encoding: .utf8)) != nil else {
            return nil
        }
        return file
    }

    /// A per-event-kind summary, so the interesting numbers do not have to be
    /// eyeballed out of hundreds of lines.
    ///
    /// Median and worst case rather than the mean: one 900 ms page is felt, and
    /// an average that has swallowed it is not.
    private static func summarise(_ snapshot: [Event]) -> String {
        var out = "Summary\n"
        out += String(repeating: "-", count: 78) + "\n"
        out += "total events : \(snapshot.count)\n"
        out += "duration     : \(snapshot.last?.atMillis ?? 0) ms\n\n"
        out += "event            count   median    p95     max   (of timed events)\n"

        let grouped = Dictionary(grouping: snapshot, by: \.kind)
        for kind in grouped.keys.sorted() {
            let group = grouped[kind] ?? []
            let timings = group.compactMap(\.durationMillis).sorted()

            out += kind.rightPadded(to: 15)
            out += String(group.count).leftPadded(to: 7)
            if timings.isEmpty {
                out += "        -        -       -\n"
            } else {
                out += String(timings[timings.count / 2]).leftPadded(to: 9)
                out += String(timings[min(timings.count * 95 / 100, timings.count - 1)])
                    .leftPadded(to: 8)
                out += String(timings[timings.count - 1]).leftPadded(to: 8)
                out += "\n"
            }
        }
        return out
    }

    /// What the device is, so a file sent by somebody else is self-describing.
    static var deviceNote: String {
        var info = utsname()
        uname(&info)
        let identifier = withUnsafeBytes(of: &info.machine) { raw in
            String(cString: raw.baseAddress!.assumingMemoryBound(to: CChar.self))
        }
        #if targetEnvironment(simulator)
        let architecture = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"]
            .map { "simulator (\($0))" } ?? "simulator"
        #else
        let architecture = identifier
        #endif
        #if canImport(UIKit)
        return "\(UIDevice.current.model) \(identifier) | iOS "
            + "\(UIDevice.current.systemVersion) | \(architecture)"
        #else
        return "\(identifier) | host | \(architecture)"
        #endif
    }

    /// Where recordings go: the app's own Documents directory.
    ///
    /// With `UIFileSharingEnabled` and `LSSupportsOpeningDocumentsInPlace` set,
    /// that is reachable from the Files app and from a Mac — the direct analogue
    /// of Android's `adb pull`-able external files directory. A file nobody can
    /// retrieve records nothing.
    static var directory: URL {
        FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
    }
}

private extension String {
    func leftPadded(to width: Int) -> String {
        count >= width ? self : String(repeating: " ", count: width - count) + self
    }

    func rightPadded(to width: Int) -> String {
        count >= width ? self : self + String(repeating: " ", count: width - count)
    }
}
