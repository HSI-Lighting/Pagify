# Building the Session Recorder into Pagify for iOS

Written for a coding agent porting Pagify to iOS. It covers what the recorder
is, why it exists, exactly how the Android one works, and what to build.

Read `docs/IOS_PORT.md` in the repo first for the wider port. This document is
only about the recorder — but build it **early**, because it is the tool you
will diagnose the rest of the port with.

---

## 1. What it is

A switch in the reader's overflow menu. Turn it on, use the app, turn it off,
and a plain text file lands in the app's own directory containing a timestamped
timeline of everything the renderer and the gestures did, followed by a summary
table of counts and timings per event type.

It is **not** analytics. Nothing is uploaded, nothing leaves the device, and it
is off unless somebody deliberately turns it on. It is an instrument for the
person holding the phone, and for whoever they send the file to.

The whole Android implementation is 127 lines
(`app/src/main/java/com/hsilighting/pagify/core/SessionRecorder.kt`). It is
worth porting exactly, because its value is entirely in being trivial to use and
trivial to read.

---

## 2. Why it exists — read this before deciding it is optional

A rendering and gesture bug on a touch device is very hard to argue about. The
person who saw it has a memory of something feeling wrong; you have code that
looks right. The recorder replaces that with numbers.

Two real cases from the Android build, both of which cost real time and both of
which the recorder settled:

**It disproved a confident diagnosis.** Dragging the first page of a document
downwards in the page organiser made the grid run away down the document. The
edge-scroll code was rewritten to fix it, with a plausible and entirely
wrong explanation. The recording then showed the grid going from page 10 to page
45 in **78 milliseconds** with **zero scroll events recorded at all** — the
scroll code had not run. The real cause was elsewhere: a lazy grid anchors its
scroll position to the key of the first visible item, and the keys were the page
numbers, so the grid was faithfully chasing the page being dragged. Without the
recording that would have been several more rounds of guessing.

**It measured what "laggy" meant.** A scroll that "felt slow" read back as
eleven thumbnails at ~240 ms each with three cache misses — which pointed
straight at the cache, not the renderer.

The lesson worth carrying into the port: **when the recording has no event for
the thing you are blaming, that is itself the finding.** Add the event, then
look again.

---

## 3. How the Android one works

### 3.1 Shape

A process-wide singleton (Kotlin `object`) holding:

- an `AtomicBoolean` — recording or not,
- a `ConcurrentLinkedQueue<Event>` — the events so far,
- `startedAtNanos` — the clock origin, so timestamps are relative to the start
  of the recording rather than to the epoch,
- `header` — the text block written above the timeline, built once at `start`.

An event is four fields, all untyped:

```kotlin
private data class Event(
    val atMillis: Long,      // since start()
    val kind: String,        // "THUMB_REQ", "PAGE_ENTER", …
    val detail: String,      // "page=12 scale=0.35"
    val durationMillis: Long?, // null for instantaneous marks
)
```

### 3.2 The three calls

```kotlin
fun start(documentName: String, pageCount: Int, deviceNote: String)
fun record(kind: String, detail: String, durationMillis: Long? = null)
fun stop(directory: File): File?   // returns the file written, or null
```

`record` is the only one call sites touch. It is deliberately cheap when idle —
one atomic read and an early return — which is what allows the hooks to stay in
release builds. **Keep that property.** A recorder that must be compiled out is
a recorder that is never on when the bug happens.

### 3.3 The file

Header, then a fixed-width timeline, then a summary.

```
Pagify session recording
document : HSI CATALOG 2026.pdf
pages    : 149
device   : SM-G736B | Android 15 | arm64-v8a
engine   : pdf_core 0.1.0

t(ms)   event            detail
------  ---------------  ------------------------------------------------------------
 11757  THUMB_REQ        page=0
 11859  THUMB_REQ        page=1
 11927  THUMB_HIT        page=4
 12006  THUMB_RENDER     page=45 px=132x187 kb=96  took=61ms
 …

Summary
------------------------------------------------------------------------------
total events : 1105
duration     : 14591 ms

event            count   median    p95     max   (of timed events)
THUMB_HIT          204        -        -       -
THUMB_REQ          165        -        -       -
THUMB_RENDER        86       48      210     318
```

Three deliberate choices, all worth keeping:

- **Plain text, fixed columns.** The file is meant to be pulled off the device
  and read immediately — by a person, or pasted to a model — with no tooling,
  no schema and no viewer. Do not make it JSON.
- **Relative timestamps in milliseconds.** What matters is the gap between two
  events, and epoch timestamps make that arithmetic by hand.
- **Median, p95 and max in the summary — not the mean.** One 900 ms page is
  felt; an average that has swallowed it is not.

### 3.4 Where the file goes

Android writes to the app's external files directory
(`getExternalFilesDir(null)`), because it needs no permission and can be pulled
with `adb pull` directly. Filenames are `pagify-session-<epochMillis>.txt`.

**iOS equivalent:** the app's `Documents` directory, with
`UIFileSharingEnabled` (Application supports iTunes file sharing) and
`LSSupportsOpeningDocumentsInPlace` set in `Info.plist` so the files are
reachable from the Files app and from a Mac. That is the direct analogue of
"pullable without ceremony", and without it the recorder is useless — a file
nobody can retrieve records nothing.

Also offer a **share sheet** on stop (`UIActivityViewController`) with the file,
so it can be sent straight from the device. On Android the file is retrieved by
cable; on iOS that is a worse assumption, so sharing matters more.

### 3.5 Starting and stopping

`PdfReaderViewModel.toggleRecording(externalFilesDir)`:

- if recording → `stop(directory)`, set `isRecording = false`, return
  `"Saved <filename>"` for a toast,
- else → `start(...)` with the document name, page count and a device note, set
  `isRecording = true`, return `"Recording"`.

The device note is
`"${Build.MODEL} | Android ${Build.VERSION.RELEASE} | ${Build.SUPPORTED_ABIS.firstOrNull()}"`.
The iOS equivalent is model identifier, iOS version, and architecture — it is
there so a file sent by someone else is self-describing.

The engine version comes from `NativeBridge.nativeVersion()` and should keep
coming from the Rust core on iOS, for the same reason.

While recording, the reader's overflow entry shows a **stop icon tinted with the
error colour** rather than the record icon, so an accidentally-left-on recorder
is visible. Keep that.

---

## 4. What to record

There are ~30 event kinds on Android and about 60 call sites. Port the
vocabulary rather than inventing one — a recording that reads the same on both
platforms is worth a great deal when a bug appears on one and not the other.

### 4.1 The current vocabulary

**Thumbnails** (the page rail and the organiser grid)
`THUMB_REQ`, `THUMB_HIT`, `THUMB_RENDER` (timed), `THUMB_SKIP`, `THUMB_FAIL`,
`THUMB_WARM`, `THUMB_WARM_DONE`

**Pages** (the reader)
`PAGE_ENTER`, `PAGE_OUTLINE`, `PAGE_PROXY`, `PAGE_READABLE`, `PAGE_PIXELS`,
`PAGE_PAINT`, `PAGE_MARKS`, `PAGE_MAPPING`, `PAGE_FAIL`, `PAGES_MEASURED`

The page ones form a *pipeline*, and reading them in order is how a slow page
gets diagnosed: entered → outline laid out → low-res proxy shown → readable
pixels arrived. A gap between two of those names the stage that is slow.

**Zoom and gestures**
`ZOOM_ENTER`, `ZOOM_TOUCH`, `ZOOM_SETTLED`, `ZOOM_DTAP_LIST`,
`ZOOM_DTAP_PINNED`, `TOOL_GESTURE`, `DRAG_SCROLL`

**Markup**
`ANNOTATION_ADD`, `ANNOTATION_CLEAR`, `MARKS_LOADED`, `MARKUP_REWRITE`,
`MARKUP_EMPTIED`, `TEXT_MARKS_LOADED`, `TEXT_EMPTIED`, `TEXT_RESTYLE`,
`TEXT_MOVE`, `HIGHLIGHT_SELECT`, `HIGHLIGHT_MISSED`, `SELECT_NO_TEXT`,
`CAPTURE_BOX`

**Documents**
`SAVED`, `SAVED_COPY`, `SAVED_ON_LEAVE`, `CREATED_BLANK`, `EXPORTED_PAGES`,
`BLANK_END`, `TRIM_MEMORY`

### 4.2 The conventions that make it readable

- **Kind is `SCREAMING_SNAKE`, ≤ 15 characters.** The column is padded to 15;
  longer names ragged the file.
- **Detail is `key=value` pairs, space separated.** `page=12 px=132x187 kb=96`.
  This is what makes the file greppable and countable with one line of `awk`,
  which is how it is actually read.
- **`page=` is always the page index**, so a whole recording can be filtered to
  one page.
- **Timed events pass `durationMillis`**; instantaneous marks leave it null. Only
  the timed ones appear in the summary's median/p95/max columns.
- **Pairs matter.** `THUMB_REQ` then `THUMB_RENDER` for the same page is a cache
  miss and its cost; `THUMB_HIT` alone is a hit. Do not collapse them.

### 4.3 Record the gestures too

The most valuable single lesson from the Android build: the gesture code was
**not** instrumented, and that is exactly where the hard bug was. A gesture
event with the numbers that drive it — speed, distance, target — turns "it feels
wrong" into a measurement.

`DRAG_SCROLL` was added for precisely this and records `speed=<px/frame>
moved=<px>`. On iOS, instrument at minimum:

- pinch/zoom: scale on begin, on change (throttled) and on settle,
- drag reorder: the speed and distance of any auto-scroll,
- page turns and any auto-scroll that moves the view without a finger.

If a gesture can surprise somebody, it should leave a trace.

---

## 5. What to build on iOS

### 5.1 The recorder itself

A Swift singleton mirroring the Kotlin one. Notes on the details that matter:

- **Thread safety.** Android uses `AtomicBoolean` +
  `ConcurrentLinkedQueue`. In Swift, a serial `DispatchQueue` guarding an array
  is fine, but keep the *idle* path lock-free: read an
  `atomic`/`OSAllocatedUnfairLock`-guarded flag, or a plain `Bool` read, and
  return before touching the queue. `record` is called from render callbacks and
  gesture handlers at frame rate; it must not become a contention point.
- **Clock.** Use a monotonic clock —
  `DispatchTime.now().uptimeNanoseconds` or `CLOCK_UPTIME_RAW`. Do **not** use
  `Date()`: it can jump backwards and produce negative gaps in a timeline.
  Android's `System.nanoTime()` is monotonic for the same reason.
- **Formatting.** Reproduce the column widths exactly (`%6d` for the
  timestamp, kind padded to 15, two spaces between columns). A file that
  diff-reads against an Android one is worth the small effort.
- **Memory.** The queue is unbounded on Android and a long session is a few
  hundred kilobytes of text, which is fine. If you cap it, drop from the
  *front* and say so in the header — silently losing the beginning of a
  timeline is worse than saying "first N events dropped".

### 5.2 The call sites

Put a `record` call at every point the Android build has one. They are found
with:

```bash
grep -rn "SessionRecorder.record" app/src/main/java --include=*.kt
```

Around 60 sites across nine files: the view model (~35), the reader screen (9),
the annotation layer (6), the page view (4), the zoomed page (3), the blank
frame detector (3), and one each in the grid reorder, two-finger pan and
application class.

### 5.3 The UI

- An entry in the reader's overflow: "Record a render timeline" / "Stop
  recording and save the render timeline".
- Tinted with the error colour and showing a stop icon while active.
- A toast (iOS: a brief banner or the share sheet) on stop, naming the file.

---

## 6. How to read a recording

The file is designed for `grep` and `awk`. The commands that actually get used:

```bash
# what kinds are in here, and how many of each
awk '{print $2}' session.txt | sort | uniq -c | sort -rn

# the timeline for one event kind, timestamp and detail only
grep THUMB_REQ session.txt | awk '{print $1, $3}'

# everything about page 12
grep "page=12" session.txt

# the slow ones
grep "took=" session.txt | sed 's/.*took=//' | sort -rn | head
```

The technique that found the drag bug: take an event that reveals the viewport
(here `THUMB_REQ`, since the grid only asks for thumbnails it is showing), print
timestamp and page, and look at the *rate of change*. Pages 15 → 47 across
149 ms is a viewport moving at roughly 25,000 px/s, which no gesture asked for.

And the technique that mattered more: **check whether the events you expected
are there at all.** Zero `DRAG_SCROLL` lines in a recording of a runaway scroll
is the entire finding.

---

## 7. Do not

- **Do not upload anything.** The file is the user's. No network, no analytics
  SDK, no crash reporter integration. Its usefulness rests on being obviously
  private.
- **Do not record continuously.** It is off until switched on, and it clears
  its buffer on `start`.
- **Do not put personal content in `detail`.** Page indices, pixel sizes,
  durations, scales, counts. Never page text, never annotation text, never file
  paths outside the app's own directory. The header carries the document name
  because the user chose to record that document; nothing else should carry
  content.
- **Do not compile the hooks out of release.** The bugs worth recording happen
  on real devices in real use.
- **Do not turn it into JSON**, or add a viewer. The moment reading it needs a
  tool, it stops being reached for.
