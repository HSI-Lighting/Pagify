//! One running Pagify, however many times it is launched.
//!
//! # Why this exists
//!
//! Double-clicking a PDF in Explorer starts `Pagify.exe` with the file on its
//! command line, every time — so a second document meant a second program, a
//! second copy of every library, and a second taskbar button for what is, to
//! the person using it, one application with two documents. They asked for the
//! document to arrive as **a new tab of the window that is already open**, and
//! for a tab to be draggable out into a window of its own when they want one.
//! This is the first half: the running Pagify takes the document in. The second
//! is `hub`: several windows in the one program, and tabs that move between them
//! — and with several windows, "the window already open" is the one used last.
//!
//! # How it is done
//!
//! There is no network and no named pipe here (the release gate refuses any
//! socket, see `tools/no_sockets.sh`), so the hand-over is **files**, in a
//! folder that is Pagify's own:
//!
//! * **The lock.** At start-up a process opens `instance.lock` so that no
//!   second open can succeed (share mode 0 on Windows, `flock` elsewhere). The
//!   process holding it is *the running instance*. The operating system lets go
//!   of it when the process ends, however it ends, so there is no stale lock
//!   to clean up after a crash — which a pid written into a file would have.
//! * **The request.** A launch that cannot take the lock writes what it was
//!   asked to do — the files on its command line and its `--run` commands —
//!   into `inbox/`, to a `.tmp` name and then **renamed** to `.req`, so a
//!   reader never sees half of one. It asks Windows to let the running
//!   instance come to the front (the launch is the one process Explorer has
//!   just given that right to), waits a few seconds for the answer and exits.
//! * **The answer.** The running instance has a watcher thread that looks in
//!   the inbox a few times a second and wakes the window; the window **claims**
//!   the request by renaming it away — which is what the launch is waiting for
//!   — and opens the files as tabs. The claim is a rename, so exactly one side
//!   wins it: the instance, or the launch giving up.
//! * **No answer, no lock-out.** If nobody claims the request in time — the
//!   running instance is hung, or is the other kind of thing that holds a lock
//!   without answering — the launch **withdraws** its request (the same rename,
//!   so the file cannot also be opened late and twice) and carries on as an
//!   ordinary independent Pagify, without the lock. A person is never left
//!   with a document that will not open.
//!
//! The lock and the inbox live under [`pagify_shell::state::state_dir`], the
//! same folder as the settings and the session log, **not** under
//! `%LOCALAPPDATA%`: a copy of Pagify started with `APPDATA` pointing
//! somewhere else (a test, a second profile) must not be able to find a
//! person's real window and hand a file to it.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::PagifyApp;

/// What a request file says it is. A reader that does not know the version
/// leaves the file alone rather than guess: the launch then times out and
/// opens the document itself, which is the worst a mismatch can cost.
const PROTOCOL: u32 = 1;
const HEADER: &str = "pagify-open";
const END: &str = "end";

/// How long a launch waits for the running instance before it gives up on it.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(3);
/// How often a waiting launch looks to see whether its request was taken.
const LOOK_EVERY: Duration = Duration::from_millis(25);
/// How often the running instance looks in the inbox. `read_dir` of a folder
/// that is nearly always empty: cheap enough to do five times a second.
const WATCH_EVERY: Duration = Duration::from_millis(200);
/// A request this old has no launch waiting for it any more (they give up after
/// [`ANSWER_WITHIN`]) — it was left by one that died. Opening it now would put a
/// document on screen that somebody asked for days ago.
const STALE_AFTER: Duration = Duration::from_secs(30);

// -- what is asked ---------------------------------------------------------

/// What one launch of Pagify was asked to do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    /// The documents on its command line, in order.
    pub files: Vec<PathBuf>,
    /// Its `--run` commands, in order.
    pub commands: Vec<String>,
}

enum Decoded {
    Request(Request),
    /// Written by a build whose protocol this one does not speak.
    Newer,
    /// Not a request: cut short, or something else entirely.
    Malformed,
}

impl Request {
    pub fn new(files: &[String], commands: &[String]) -> Self {
        Request { files: files.iter().map(PathBuf::from).collect(), commands: commands.to_vec() }
    }

    /// The same request with every file made absolute.
    ///
    /// **The running instance does not share this launch's working directory**,
    /// so `pagify notes.pdf` typed in a shell would be looked for in whatever
    /// folder the running one happened to be started from. Explorer always
    /// passes absolute paths; a shell does not.
    fn absolute(&self) -> Self {
        Request {
            files: self.files.iter().map(|f| std::path::absolute(f).unwrap_or_else(|_| f.clone())).collect(),
            commands: self.commands.clone(),
        }
    }

    /// One line per fact, `end` last — so a file that stops short is
    /// recognisably cut off rather than a shorter request.
    fn encode(&self) -> String {
        let mut text = format!("{HEADER} {PROTOCOL}\n");
        for file in &self.files {
            text.push_str(&format!("file {}\n", escape(&file.to_string_lossy())));
        }
        for command in &self.commands {
            text.push_str(&format!("run {}\n", escape(command)));
        }
        text.push_str(END);
        text.push('\n');
        text
    }

    fn decode(text: &str) -> Decoded {
        let mut lines = text.lines();
        let Some(version) = lines.next().and_then(|l| l.strip_prefix(HEADER)).map(str::trim) else {
            return Decoded::Malformed;
        };
        match version.parse::<u32>() {
            Ok(PROTOCOL) => {}
            Ok(_) => return Decoded::Newer,
            Err(_) => return Decoded::Malformed,
        }
        let mut request = Request::default();
        let mut ended = false;
        for line in lines {
            if ended {
                // Nothing follows `end`.
                return Decoded::Malformed;
            }
            if line == END {
                ended = true;
            } else if let Some(file) = line.strip_prefix("file ") {
                match unescape(file) {
                    Some(file) if !file.is_empty() => request.files.push(PathBuf::from(file)),
                    _ => return Decoded::Malformed,
                }
            } else if let Some(command) = line.strip_prefix("run ") {
                match unescape(command) {
                    Some(command) => request.commands.push(command),
                    None => return Decoded::Malformed,
                }
            } else {
                return Decoded::Malformed;
            }
        }
        if ended { Decoded::Request(request) } else { Decoded::Malformed }
    }
}

/// Backslash, line feed and carriage return escaped, so one fact is one line.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r")
}

/// The reverse of [`escape`]; `None` for an escape that was never written.
fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            '\\' => '\\',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(out)
}

// -- the lock ---------------------------------------------------------------

pub enum Lock {
    /// This process is now the running instance. Dropping the file lets go.
    Held(File),
    /// Another live process holds it.
    Taken,
    /// The lock could not be tried at all — an unwritable folder, say.
    /// **Not** the same as taken: with nothing to hand a request to, the
    /// only thing that is not a lock-out is carrying on alone.
    Unavailable,
}

/// Try to become the running instance.
///
/// Share mode 0 on Windows: while this handle is open nothing else can open the
/// file at all, and the handle is closed by the system when the process ends —
/// so the lock goes with a crash, and goes before anything waiting on the
/// process (the update script, `tasklist`) sees it gone. **Not inherited** by a
/// child process: `std` opens files non-inheritable, which is what lets the
/// update script outlive this process without holding the lock.
#[cfg(windows)]
pub fn take_lock(path: &Path) -> Lock {
    use std::os::windows::fs::OpenOptionsExt;
    match OpenOptions::new().write(true).create(true).truncate(false).share_mode(0).open(path) {
        Ok(file) => Lock::Held(file),
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
        Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => Lock::Taken,
        Err(_) => Lock::Unavailable,
    }
}

#[cfg(not(windows))]
pub fn take_lock(path: &Path) -> Lock {
    match OpenOptions::new().write(true).create(true).truncate(false).open(path) {
        Ok(file) => match file.try_lock() {
            Ok(()) => Lock::Held(file),
            Err(fs::TryLockError::WouldBlock) => Lock::Taken,
            Err(_) => Lock::Unavailable,
        },
        Err(_) => Lock::Unavailable,
    }
}

/// Let the running instance take the foreground.
///
/// **Windows decides who may.** A process that was not in the foreground cannot
/// raise its window, and the running Pagify usually is not: the launch is the
/// process Explorer just gave that right to, and this passes it on (to any
/// process, which is all there is to name — the launch does not know which
/// window will answer).
#[cfg(windows)]
fn allow_foreground() {
    #[link(name = "user32")]
    extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
    }
    // ASFW_ANY is `(DWORD)-1`.
    unsafe {
        AllowSetForegroundWindow(u32::MAX);
    }
}

#[cfg(not(windows))]
fn allow_foreground() {}

// -- the launch's side --------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum Forwarded {
    /// The running instance took it.
    Delivered,
    /// Nobody did, and the request is withdrawn: the launch must do it itself.
    Unanswered,
}

static SERIAL: AtomicU32 = AtomicU32::new(0);

/// Put `request` in `inbox` so that a reader can never see part of it.
fn write_request(inbox: &Path, request: &Request) -> io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Sorts in the order launches happened; the pid and counter keep two
    // launches in the same instant apart.
    let stem = format!("{nanos:024}-{}-{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed));
    let partial = inbox.join(format!("{stem}.tmp"));
    let ready = inbox.join(format!("{stem}.req"));
    pagify_shell::state::write_own(&partial, request.encode().as_bytes())?;
    if let Err(e) = fs::rename(&partial, &ready) {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    Ok(ready)
}

/// Hand `request` to the running instance and wait up to `wait` for it to say
/// it has it — by claiming the file, see [`claim_all`].
pub fn forward(inbox: &Path, request: &Request, wait: Duration) -> Forwarded {
    let Ok(sent) = write_request(inbox, request) else { return Forwarded::Unanswered };
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if is_gone(&sent) {
            return Forwarded::Delivered;
        }
        std::thread::sleep(LOOK_EVERY);
    }
    withdraw(&sent)
}

fn is_gone(path: &Path) -> bool {
    matches!(fs::metadata(path), Err(e) if e.kind() == io::ErrorKind::NotFound)
}

/// Take a request back, unless the running instance got there first.
///
/// **A rename, not a delete, and the same rename the running instance claims
/// with.** A delete would leave a moment in which the instance had read the
/// file and not yet claimed it, and the document would open twice — once there,
/// once here. With one rename there is exactly one winner, and the loser finds
/// the file gone.
fn withdraw(sent: &Path) -> Forwarded {
    let aside = sent.with_extension("withdrawn");
    match fs::rename(sent, &aside) {
        Ok(()) => {
            let _ = fs::remove_file(&aside);
            Forwarded::Unanswered
        }
        // Claimed in the last instant: it was answered after all.
        Err(e) if e.kind() == io::ErrorKind::NotFound => Forwarded::Delivered,
        Err(_) => Forwarded::Unanswered,
    }
}

// -- start-up -----------------------------------------------------------------

/// This process holds the lock and answers the inbox.
pub struct Primary {
    lock: File,
    inbox: PathBuf,
}

pub enum Start {
    /// First Pagify: open a window, and answer others.
    Primary(Primary),
    /// Another Pagify took the request: open nothing, exit.
    Forwarded,
    /// No lock and no one to ask — carry on as a Pagify of its own.
    Independent,
}

/// Decide what this launch is: the running instance, a request to it, or alone.
pub fn start(dir: &Path, request: &Request, wait: Duration) -> Start {
    if fs::create_dir_all(dir).is_err() {
        return Start::Independent;
    }
    let inbox = dir.join("inbox");
    match take_lock(&dir.join("instance.lock")) {
        Lock::Held(lock) => Start::Primary(Primary { lock, inbox }),
        Lock::Unavailable => Start::Independent,
        Lock::Taken => {
            allow_foreground();
            match forward(&inbox, &request.absolute(), wait) {
                Forwarded::Delivered => Start::Forwarded,
                Forwarded::Unanswered => Start::Independent,
            }
        }
    }
}

// -- the running instance's side ---------------------------------------------

fn is_request(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "req")
}

/// Whether anything that looks like a request is waiting. Cheap.
fn has_request(inbox: &Path) -> bool {
    fs::read_dir(inbox).is_ok_and(|entries| entries.flatten().any(|e| is_request(&e.path())))
}

/// Take every request in the inbox, oldest first.
///
/// **Claiming is the answer.** The file is renamed out of the inbox before it
/// is used; a launch waiting on it sees it gone and exits. A request that cannot
/// be claimed was withdrawn by its launch in the same instant, and is not ours.
///
/// What is not a request is left alone — a `.tmp` is somebody still writing —
/// except that anything that has sat for [`STALE_AFTER`] is cleared, since by
/// then nobody is writing it or waiting for it.
fn claim_all(inbox: &Path) -> Vec<Request> {
    let mut found: Vec<PathBuf> = match fs::read_dir(inbox) {
        Ok(entries) => entries.flatten().map(|e| e.path()).collect(),
        Err(_) => return Vec::new(),
    };
    found.sort();
    let mut taken = Vec::new();
    for path in found {
        let stale = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_AFTER);
        if stale {
            let _ = fs::remove_file(&path);
            continue;
        }
        if !is_request(&path) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let request = match Request::decode(&text) {
            Decoded::Request(request) => request,
            // Not for this build to answer.
            Decoded::Newer => continue,
            // Not a request, and nothing will ever make it one.
            Decoded::Malformed => {
                let _ = fs::remove_file(&path);
                continue;
            }
        };
        let claimed = path.with_extension("taken");
        if fs::rename(&path, &claimed).is_err() {
            continue;
        }
        let _ = fs::remove_file(&claimed);
        taken.push(request);
    }
    taken
}

/// What the running window keeps of all this: the lock, the inbox, and which
/// window a document should go to.
pub struct Handover {
    inbox: Option<PathBuf>,
    /// Set by the watcher thread, taken by the window.
    waiting: Arc<AtomicBool>,
    /// The window the person last used — where a document handed over goes.
    ///
    /// Tracked from the first window so that several windows need nothing more
    /// than to report their own focus to [`Handover::note_focus`].
    last_used: egui::ViewportId,
    /// Held for as long as the program runs.
    _lock: Option<File>,
}

impl Default for Handover {
    /// Not the running instance — nothing is answered. What every test starts
    /// with, and what a launch that could not take the lock runs as.
    fn default() -> Self {
        Handover {
            inbox: None,
            waiting: Arc::new(AtomicBool::new(false)),
            last_used: egui::ViewportId::ROOT,
            _lock: None,
        }
    }
}

impl Handover {
    /// Start answering the inbox, waking `ctx` when something arrives.
    ///
    /// The thread only *notices*. Claiming is left to the window, so that an
    /// answer means the window really is running: a hung one stops claiming and
    /// its launches stop waiting for it.
    pub fn watch(primary: Primary, ctx: egui::Context) -> Self {
        Self::watch_every(primary, ctx, WATCH_EVERY)
    }

    fn watch_every(primary: Primary, ctx: egui::Context, every: Duration) -> Self {
        // True to begin with: a launch that arrived while this one was still
        // opening its window is already in the folder.
        let waiting = Arc::new(AtomicBool::new(true));
        let flag = waiting.clone();
        let inbox = primary.inbox.clone();
        let _ = std::thread::Builder::new().name("pagify-inbox".into()).spawn(move || loop {
            std::thread::sleep(every);
            if has_request(&inbox) {
                flag.store(true, Ordering::Release);
                // Also what gets a minimised window to run a frame.
                ctx.request_repaint();
            }
        });
        Handover {
            inbox: Some(primary.inbox),
            waiting,
            last_used: egui::ViewportId::ROOT,
            _lock: Some(primary.lock),
        }
    }

    /// Everything handed over since this was last asked.
    pub(crate) fn take(&self) -> Vec<Request> {
        let Some(inbox) = &self.inbox else { return Vec::new() };
        // Cleared **before** looking, so a request written while this runs sets
        // it again and is found on the next frame rather than lost.
        if !self.waiting.swap(false, Ordering::AcqRel) {
            return Vec::new();
        }
        claim_all(inbox)
    }

    /// Record that window `id` has the focus.
    pub fn note_focus(&mut self, id: egui::ViewportId) {
        self.last_used = id;
    }

    /// The window a document handed over from outside should open in.
    pub fn target_window(&self) -> egui::ViewportId {
        self.last_used
    }
}

impl PagifyApp {
    /// Open what other launches of Pagify have handed over, as new tabs of the
    /// window used last, and bring that window forward. Called from `logic`, so
    /// that it runs for a minimised window too; a frame with nothing waiting
    /// costs one atomic load.
    pub(crate) fn take_handover(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().focused) == Some(true) {
            self.handover.note_focus(ctx.viewport_id());
        }
        for request in self.handover.take() {
            let window = self.handover.target_window();
            self.open_handed_over(&request, ctx, window);
        }
    }

    /// Open what `request` asks for in this window, and bring `window` — the
    /// viewport this app is drawn in — forward. With several windows the `Hub`
    /// calls this on the one used last; alone, that is only ever the first.
    pub(crate) fn open_handed_over(&mut self, request: &Request, ctx: &egui::Context, window: egui::ViewportId) {
        // **A question that is up stays up.** "Close this document?" is drawn
        // for the active tab alone; a document that arrived and took the focus
        // away from it would hide the question until somebody found their way
        // back. The document still opens, as a tab, and the window still comes
        // forward — it just does not take the active place.
        let asking = self.tab().closing.is_some();
        // In the session log, so that a document that "opened by itself" can be
        // told from one that was opened here.
        self.say_info(match request.files.len() {
            0 => "another launch of Pagify brought this window forward.".to_string(),
            n => format!("another launch of Pagify handed over {n} file{}.", if n == 1 { "" } else { "s" }),
        });
        for file in &request.files {
            match self.tab_showing(file) {
                // **Already open: shown, not opened again.** A second copy of a
                // document with unsaved edits is two documents that disagree, and
                // saving either throws the other's work away.
                Some(index) => {
                    self.active_tab = index;
                    self.say_info(format!("{} is already open.", file.display()));
                }
                None => self.open(&file.to_string_lossy()),
            }
        }
        for command in &request.commands {
            self.submit(command);
        }
        if asking {
            // Found again by the question itself: the new tabs went in at the
            // front, so the place it had is not the place it has.
            if let Some(index) = self.tabs.iter().position(|t| t.closing.is_some()) {
                self.active_tab = index;
            }
        }
        ctx.send_viewport_cmd_to(window, egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(window, egui::ViewportCommand::Focus);
    }

    /// The tab that already has `path` open, if any.
    ///
    /// Compared by what the path *is* rather than how it was typed: Explorer
    /// says `C:\Docs\A.pdf`, a recent-files entry says `c:/docs/a.pdf`.
    pub(crate) fn tab_showing(&self, path: &Path) -> Option<usize> {
        let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let wanted = real(path);
        self.tabs
            .iter()
            .position(|t| t.doc.as_ref().is_some_and(|d| real(d.session.path()) == wanted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_tests::{fixture, harness_from};
    use crate::{Closing, DocTab};

    /// A scratch folder of this test's own, empty.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pagify-instance-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    fn asking_for(files: &[&str]) -> Request {
        Request { files: files.iter().map(PathBuf::from).collect(), commands: Vec::new() }
    }

    /// Stand in for the running instance's window: claim whatever arrives.
    fn answer_after(inbox: &Path, delay: Duration) -> std::thread::JoinHandle<Vec<Request>> {
        let inbox = inbox.to_path_buf();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            claim_all(&inbox)
        })
    }

    // -- the file format ------------------------------------------------------

    #[test]
    fn a_request_survives_being_written_down_and_read_back() {
        let request = Request {
            files: vec![
                PathBuf::from(r"C:\Users\Ann Lee\My Documents\plan (v2).pdf"),
                PathBuf::from("/home/zoë/日本語.pdf"),
                PathBuf::from(r"C:\odd\back\\slash\n.pdf"),
            ],
            commands: vec!["thumbnails".into(), "say one\ntwo\r\nthree\\n".into(), String::new()],
        };
        match Request::decode(&request.encode()) {
            Decoded::Request(back) => assert_eq!(back, request),
            _ => panic!("a request did not come back as one"),
        }
        match Request::decode(&Request::default().encode()) {
            Decoded::Request(back) => assert_eq!(back, Request::default()),
            _ => panic!("an empty request (a bare launch) did not come back"),
        }
    }

    #[test]
    fn a_request_that_is_cut_short_or_not_one_is_malformed() {
        let whole = asking_for(&["a.pdf", "b.pdf"]).encode();
        // Every proper prefix of a whole request is cut off — never a shorter request.
        // (Without its last line feed it is still a whole request, so stop short of that.)
        for cut in 0..whole.len() - 1 {
            if !whole.is_char_boundary(cut) {
                continue;
            }
            assert!(
                matches!(Request::decode(&whole[..cut]), Decoded::Malformed),
                "a request cut at byte {cut} was taken for a whole one: {:?}",
                &whole[..cut]
            );
        }
        for bad in [
            "hello\nend\n",
            "pagify-open x\nend\n",
            "pagify-open 1\nfile a.pdf\nbogus line\nend\n",
            "pagify-open 1\nfile \nend\n",
            "pagify-open 1\nfile a\\qb\nend\n",
            "pagify-open 1\nfile a.pdf\\\nend\n",
            "pagify-open 1\nend\nfile a.pdf\n",
        ] {
            assert!(matches!(Request::decode(bad), Decoded::Malformed), "{bad:?} was accepted");
        }
        assert!(matches!(Request::decode("pagify-open 99\nfile a.pdf\nend\n"), Decoded::Newer));
    }

    #[test]
    fn a_relative_file_is_made_absolute_before_it_is_handed_over() {
        let request = asking_for(&["notes.pdf"]).absolute();
        assert!(request.files[0].is_absolute(), "{:?}", request.files[0]);
        assert!(request.files[0].ends_with("notes.pdf"));
    }

    // -- the inbox ------------------------------------------------------------

    #[test]
    fn a_request_is_written_whole_and_never_visible_half_done() {
        let inbox = scratch("atomic");
        let sent = write_request(&inbox, &asking_for(&["a.pdf"])).expect("written");
        assert!(is_request(&sent));
        let names: Vec<_> = fs::read_dir(&inbox).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 1, "a temporary file was left beside the request: {names:?}");
        assert!(matches!(Request::decode(&fs::read_to_string(&sent).unwrap()), Decoded::Request(_)));
        let _ = fs::remove_dir_all(&inbox);
    }

    #[test]
    fn the_reader_skips_what_is_still_being_written_and_drops_what_is_not_a_request() {
        let inbox = scratch("reader");
        // Half of one, as a writer in the middle of its write leaves it.
        fs::write(inbox.join("0001.tmp"), "pagify-open 1\nfile a.pdf\n").unwrap();
        // Cut short under its final name — which no writer here produces, but a
        // reader that opened it would be opening half a request.
        fs::write(inbox.join("0002.req"), "pagify-open 1\nfile b.pdf\n").unwrap();
        fs::write(inbox.join("0003.req"), "not a request at all").unwrap();
        // Another build's.
        fs::write(inbox.join("0004.req"), "pagify-open 7\nfile d.pdf\nend\n").unwrap();
        fs::write(inbox.join("0005.req"), asking_for(&["e.pdf"]).encode()).unwrap();

        let taken = claim_all(&inbox);
        assert_eq!(taken, vec![asking_for(&["e.pdf"])], "only the whole request is for this build");
        assert!(inbox.join("0001.tmp").exists(), "a writer's own file was taken from under it");
        assert!(!inbox.join("0002.req").exists() && !inbox.join("0003.req").exists());
        assert!(inbox.join("0004.req").exists(), "a request of a newer protocol was answered by an older build");
        assert!(!inbox.join("0005.req").exists(), "claiming must take the request out of the inbox");
        assert!(claim_all(&inbox).is_empty(), "a request was claimed twice");
        let _ = fs::remove_dir_all(&inbox);
    }

    #[test]
    fn requests_are_taken_in_the_order_they_were_made() {
        let inbox = scratch("order");
        for name in ["one.pdf", "two.pdf", "three.pdf"] {
            write_request(&inbox, &asking_for(&[name])).unwrap();
        }
        let order: Vec<_> = claim_all(&inbox).into_iter().flat_map(|r| r.files).collect();
        assert_eq!(order, ["one.pdf", "two.pdf", "three.pdf"].map(PathBuf::from));
        let _ = fs::remove_dir_all(&inbox);
    }

    #[test]
    fn a_request_left_by_a_launch_that_died_is_not_opened_days_later() {
        let inbox = scratch("stale");
        let old = inbox.join("0001.req");
        fs::write(&old, asking_for(&["ancient.pdf"]).encode()).unwrap();
        let leftover = inbox.join("0002.taken");
        fs::write(&leftover, "x").unwrap();
        let long_ago = SystemTime::now() - Duration::from_secs(3600);
        for path in [&old, &leftover] {
            OpenOptions::new().write(true).open(path).unwrap().set_modified(long_ago).unwrap();
        }
        assert!(claim_all(&inbox).is_empty(), "a request from an hour ago was opened");
        assert!(!old.exists() && !leftover.exists(), "stale files were left to build up");
        let _ = fs::remove_dir_all(&inbox);
    }

    #[test]
    fn an_empty_or_missing_inbox_has_nothing_waiting() {
        let inbox = scratch("empty");
        assert!(!has_request(&inbox));
        assert!(claim_all(&inbox).is_empty());
        assert!(!has_request(&inbox.join("does-not-exist")));
        assert!(claim_all(&inbox.join("does-not-exist")).is_empty());
        fs::write(inbox.join("x.tmp"), "").unwrap();
        assert!(!has_request(&inbox), "a half-written file is not a request");
        let _ = fs::remove_dir_all(&inbox);
    }

    // -- the lock -------------------------------------------------------------

    #[test]
    fn while_one_process_holds_the_lock_a_second_attempt_fails() {
        let dir = scratch("lock");
        let path = dir.join("instance.lock");
        let first = take_lock(&path);
        assert!(matches!(first, Lock::Held(_)), "the first attempt did not get the lock");
        assert!(matches!(take_lock(&path), Lock::Taken), "a second attempt got a lock already held");
        assert!(matches!(take_lock(&path), Lock::Taken));
        drop(first);
        assert!(matches!(take_lock(&path), Lock::Held(_)), "the lock was not released when its holder let go");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unusable_lock_is_not_mistaken_for_a_held_one() {
        let dir = scratch("unusable");
        // A folder where the file should be: cannot be opened for writing.
        let path = dir.join("instance.lock");
        fs::create_dir(&path).unwrap();
        assert!(matches!(take_lock(&path), Lock::Unavailable));
        let _ = fs::remove_dir_all(&dir);
    }

    /// The update script is started by the running Pagify and outlives it. If it
    /// inherited the lock the relaunched Pagify would find the old one still "running".
    #[test]
    fn a_child_process_does_not_keep_the_lock_alive() {
        let dir = scratch("inherit");
        let path = dir.join("instance.lock");
        let Lock::Held(file) = take_lock(&path) else { panic!("no lock") };
        let mut command = if cfg!(windows) {
            let mut c = std::process::Command::new("ping");
            c.args(["-n", "6", "127.0.0.1"]);
            // No console of its own. With Windows Terminal as the default
            // terminal, every console program a test starts opens a tab, and a
            // killed one leaves it open: a few hundred test runs left a
            // hundred empty terminals behind.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            c
        } else {
            let mut c = std::process::Command::new("sleep");
            c.arg("5");
            c
        };
        let mut child = command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("a child to start");
        drop(file);
        let got = take_lock(&path);
        let _ = child.kill();
        let _ = child.wait();
        assert!(matches!(got, Lock::Held(_)), "a running child process kept the lock after its parent let go");
        let _ = fs::remove_dir_all(&dir);
    }

    // -- the launch's wait ----------------------------------------------------

    #[test]
    fn a_launch_is_told_delivered_when_the_running_instance_takes_its_request() {
        let inbox = scratch("delivered");
        let answerer = answer_after(&inbox, Duration::from_millis(150));
        let started = Instant::now();
        let outcome = forward(&inbox, &asking_for(&["a.pdf"]), Duration::from_secs(5));
        assert_eq!(outcome, Forwarded::Delivered);
        assert!(started.elapsed() < Duration::from_secs(3), "it waited out the clock instead of noticing the answer");
        assert_eq!(answerer.join().unwrap(), vec![asking_for(&["a.pdf"])]);
        let _ = fs::remove_dir_all(&inbox);
    }

    #[test]
    fn a_launch_nobody_answers_takes_its_request_back_and_does_it_itself() {
        let inbox = scratch("unanswered");
        let started = Instant::now();
        let outcome = forward(&inbox, &asking_for(&["a.pdf"]), Duration::from_millis(200));
        assert_eq!(outcome, Forwarded::Unanswered);
        assert!(started.elapsed() >= Duration::from_millis(200), "it gave up before the time was up");
        assert!(
            claim_all(&inbox).is_empty() && !has_request(&inbox),
            "the withdrawn request is still there for a hung instance to open late, and twice"
        );
        let _ = fs::remove_dir_all(&inbox);
    }

    #[test]
    fn a_request_claimed_in_the_last_instant_counts_as_answered_not_withdrawn() {
        let inbox = scratch("last-instant");
        let sent = write_request(&inbox, &asking_for(&["a.pdf"])).unwrap();
        assert_eq!(claim_all(&inbox).len(), 1);
        // The launch's clock runs out just after the claim.
        assert_eq!(withdraw(&sent), Forwarded::Delivered);
        let _ = fs::remove_dir_all(&inbox);
    }

    #[test]
    fn a_launch_that_cannot_write_its_request_does_not_wait_for_anything() {
        let dir = scratch("unwritable");
        let blocker = dir.join("inbox");
        fs::write(&blocker, "a file where the folder should be").unwrap();
        let started = Instant::now();
        assert_eq!(forward(&blocker, &asking_for(&["a.pdf"]), Duration::from_secs(5)), Forwarded::Unanswered);
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = fs::remove_dir_all(&dir);
    }

    // -- start-up -------------------------------------------------------------

    #[test]
    fn the_first_launch_is_the_running_instance_and_the_next_hands_over_to_it() {
        let dir = scratch("start");
        let first = start(&dir, &Request::default(), Duration::from_millis(100));
        assert!(matches!(first, Start::Primary(_)));

        let answerer = answer_after(&dir.join("inbox"), Duration::from_millis(100));
        let second = start(&dir, &asking_for(&["b.pdf"]), Duration::from_secs(5));
        assert!(matches!(second, Start::Forwarded), "a second launch opened a window of its own");
        let handed = answerer.join().unwrap();
        assert_eq!(handed.len(), 1);
        assert!(handed[0].files[0].is_absolute() && handed[0].files[0].ends_with("b.pdf"));
        drop(first);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_launch_beside_an_instance_that_never_answers_carries_on_alone() {
        let dir = scratch("no-answer");
        let hung = take_lock(&dir.join("instance.lock"));
        assert!(matches!(hung, Lock::Held(_)));
        let started = Instant::now();
        let outcome = start(&dir, &asking_for(&["b.pdf"]), Duration::from_millis(300));
        assert!(matches!(outcome, Start::Independent), "a launch beside a silent instance was locked out");
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(!has_request(&dir.join("inbox")), "the request was left behind");
        drop(hung);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_launch_that_cannot_use_its_folder_carries_on_alone() {
        let dir = scratch("no-folder");
        let blocker = dir.join("not-a-folder");
        fs::write(&blocker, "x").unwrap();
        assert!(matches!(start(&blocker, &Request::default(), Duration::from_secs(5)), Start::Independent));
        let _ = fs::remove_dir_all(&dir);
    }

    /// The update script relaunches Pagify the moment the old one is gone.
    #[test]
    fn a_launch_straight_after_the_running_instance_ends_becomes_the_running_instance() {
        let dir = scratch("relaunch");
        let old = start(&dir, &Request::default(), Duration::from_millis(100));
        assert!(matches!(old, Start::Primary(_)));
        drop(old);
        for _ in 0..5 {
            let next = start(&dir, &Request::default(), Duration::from_millis(100));
            assert!(matches!(next, Start::Primary(_)), "the relaunched Pagify found the old lock still held");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    // -- the window's side ----------------------------------------------------

    #[test]
    fn the_window_used_last_is_where_a_document_goes() {
        let mut handover = Handover::default();
        assert_eq!(handover.target_window(), egui::ViewportId::ROOT);
        let other = egui::ViewportId::from_hash_of("a second window");
        handover.note_focus(other);
        assert_eq!(handover.target_window(), other);
        handover.note_focus(egui::ViewportId::ROOT);
        assert_eq!(handover.target_window(), egui::ViewportId::ROOT);
    }

    /// A running window, answering a scratch inbox quickly.
    fn answering(
        h: &mut egui_kittest::Harness<'static, PagifyApp>,
        dir: &Path,
    ) -> PathBuf {
        let Lock::Held(lock) = take_lock(&dir.join("instance.lock")) else { panic!("no lock") };
        let inbox = dir.join("inbox");
        let ctx = h.ctx.clone();
        h.state_mut().handover =
            Handover::watch_every(Primary { lock, inbox: inbox.clone() }, ctx, Duration::from_millis(20));
        inbox
    }

    /// One frame the way eframe runs it: `logic`, then `ui`.
    fn frame(h: &mut egui_kittest::Harness<'static, PagifyApp>) {
        let ctx = h.ctx.clone();
        eframe::App::logic(h.state_mut(), &ctx, &mut eframe::Frame::_new_kittest());
        h.step();
    }

    /// Frames until `done`, giving the watcher thread real time to notice.
    fn run_until(h: &mut egui_kittest::Harness<'static, PagifyApp>, done: impl Fn(&PagifyApp) -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            frame(h);
            if done(h.state()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn running(first: &str) -> egui_kittest::Harness<'static, PagifyApp> {
        let app = PagifyApp::new(Some(&fixture(first)));
        assert!(app.tab().doc.is_some(), "{first} did not open");
        harness_from(app)
    }

    #[test]
    fn a_document_handed_over_arrives_as_a_new_active_tab_and_is_acknowledged() {
        let dir = scratch("window-tab");
        let mut h = running("single-page.pdf");
        let inbox = answering(&mut h, &dir);
        assert_eq!(h.state().tabs.len(), 1);

        let sent = write_request(&inbox, &asking_for(&[&fixture("two-column.pdf")])).unwrap();
        assert!(run_until(&mut h, |app| app.tabs.len() == 2), "the document never became a tab");

        let app = h.state();
        assert_eq!(app.active_tab, 0, "the new tab was not made the active one");
        let name = |t: &DocTab| t.doc.as_ref().map(|d| d.session.path().file_name().unwrap().to_string_lossy().into_owned());
        // The newest tab is the leftmost.
        assert_eq!(name(&app.tabs[1]).as_deref(), Some("single-page.pdf"), "the open document was replaced");
        assert_eq!(name(&app.tabs[0]).as_deref(), Some("two-column.pdf"));
        assert!(is_gone(&sent), "the request was never acknowledged");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_document_that_is_already_open_is_shown_not_opened_again() {
        let dir = scratch("window-dupe");
        let mut h = running("single-page.pdf");
        let inbox = answering(&mut h, &dir);
        // A second document, then ask for the first again — spelled differently.
        h.state_mut().open(&fixture("two-column.pdf"));
        assert_eq!(h.state().tabs.len(), 2);
        assert_eq!(h.state().active_tab, 0);

        let spelled = fixture("single-page.pdf").replace('/', if cfg!(windows) { "\\" } else { "/" });
        let sent = write_request(&inbox, &asking_for(&[&spelled])).unwrap();
        assert!(run_until(&mut h, |_| is_gone(&sent)), "the request was not taken");
        h.step();
        assert_eq!(h.state().tabs.len(), 2, "an open document was opened a second time");
        assert_eq!(h.state().active_tab, 1, "the tab already showing it was not brought forward");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_document_that_will_not_open_says_so_and_closes_nothing() {
        let dir = scratch("window-bad");
        let mut h = running("single-page.pdf");
        let inbox = answering(&mut h, &dir);
        let errors = h.state().ui_state.errors_said;
        let sent = write_request(&inbox, &asking_for(&["definitely-not-here.pdf"])).unwrap();
        assert!(run_until(&mut h, |_| is_gone(&sent)), "the request was not taken");
        h.step();
        let app = h.state();
        assert!(app.ui_state.errors_said > errors, "a document that failed to open said nothing");
        assert_eq!(app.tabs.len(), 1, "a failed open added or removed a tab");
        assert!(app.tab().doc.is_some(), "a failed open closed the document that was open");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_password_protected_document_asks_for_its_password_and_then_opens_as_a_tab() {
        let dir = scratch("window-password");
        let mut h = running("single-page.pdf");
        let inbox = answering(&mut h, &dir);
        let sent = write_request(&inbox, &asking_for(&[&fixture("encrypted.pdf")])).unwrap();
        assert!(run_until(&mut h, |_| is_gone(&sent)), "the request was not taken");
        h.step();
        let app = h.state_mut();
        assert!(app.tab_mut().secure_state.awaiting_password.is_some(), "an encrypted file did not ask for its password");
        assert_eq!(app.tabs.len(), 1, "a tab was made for a file that has not been opened yet");

        // The existing flow: the next line typed is the password (see `consume_password_line`).
        app.cmd.input_mut().push_str("pagify");
        assert!(app.consume_password_line());
        assert_eq!(app.tabs.len(), 2, "the document did not open once its password was given");
        assert_eq!(app.active_tab, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bare_launch_changes_no_tab_and_still_counts_as_answered() {
        let dir = scratch("window-bare");
        let mut h = running("single-page.pdf");
        let inbox = answering(&mut h, &dir);
        let sent = write_request(&inbox, &Request::default()).unwrap();
        assert!(run_until(&mut h, |_| is_gone(&sent)), "a launch with no file was never answered");
        h.step();
        assert_eq!(h.state().tabs.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_question_about_unsaved_work_is_not_buried_by_a_document_arriving() {
        let dir = scratch("window-asking");
        let mut h = running("single-page.pdf");
        let inbox = answering(&mut h, &dir);
        h.state_mut().submit("l 30,30 200,200");
        h.state_mut().submit("close");
        assert_eq!(h.state().tab().closing, Some(Closing::Document), "the question was not asked");

        let sent = write_request(&inbox, &asking_for(&[&fixture("two-column.pdf")])).unwrap();
        assert!(run_until(&mut h, |app| app.tabs.len() == 2), "the document never became a tab");
        assert!(is_gone(&sent));
        let app = h.state();
        assert_eq!(app.active_tab, 1, "the document took the place of the question");
        assert_eq!(app.tabs[1].closing, Some(Closing::Document), "the question was dropped");
        assert!(app.tabs[1].doc.is_some(), "the document with unsaved marks was closed");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn commands_in_a_request_run_after_its_documents_open() {
        let dir = scratch("window-run");
        let mut h = running("single-page.pdf");
        let inbox = answering(&mut h, &dir);
        let request = Request {
            files: vec![PathBuf::from(fixture("two-column.pdf"))],
            commands: vec!["l 30,30 200,200".into()],
        };
        let sent = write_request(&inbox, &request).unwrap();
        assert!(run_until(&mut h, |_| is_gone(&sent)));
        h.step();
        let app = h.state();
        assert_eq!(app.tabs.len(), 2);
        // The line was drawn on the new tab, not the one that was already open.
        assert_eq!(app.tabs[0].markup.existing(0).map(|l| l.len()), Some(1));
        assert_eq!(app.tabs[1].markup.existing(0).map(|l| l.len()), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// **Measured, not assumed:** with the window minimised eframe runs no egui
    /// pass, only `logic`. A document taken in `ui` was never taken, the launch
    /// timed out, and a second window opened.
    #[test]
    fn a_minimised_window_takes_the_document_with_no_ui_pass_at_all() {
        let dir = scratch("window-minimised");
        let mut h = running("single-page.pdf");
        let inbox = answering(&mut h, &dir);
        let sent = write_request(&inbox, &asking_for(&[&fixture("two-column.pdf")])).unwrap();
        let ctx = h.ctx.clone();
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end && h.state().tabs.len() < 2 {
            // `logic` alone, no `step`: what a minimised window gets.
            eframe::App::logic(h.state_mut(), &ctx, &mut eframe::Frame::_new_kittest());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(h.state().tabs.len(), 2, "a window with no ui pass never took the document");
        assert_eq!(h.state().active_tab, 0);
        assert!(is_gone(&sent));
        let _ = fs::remove_dir_all(&dir);
    }
}
