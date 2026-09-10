//! Files handed to the application by Finder.
//!
//! # Why this exists
//!
//! **Double-clicking a document does not put it in `argv`.** macOS launches the
//! application and then sends it an Apple Event — `kAEOpenDocuments` — carrying
//! the files. An application that does not answer that event gets answered
//! *for* it, and the person is told:
//!
//! ```text
//! The document "…" could not be opened.
//! Pagify cannot open files in the "PDF document" format.
//! ```
//!
//! which is macOS quoting this bundle's own `CFBundleTypeName` back at them.
//! Reported from use exactly that way, and it applied to every PDF rather than
//! to any particular one.
//!
//! # How it is done
//!
//! The event handler runs on the main thread, inside AppKit, at a moment the
//! rest of the program knows nothing about. So it does the least it can: push
//! the paths onto a queue. The application drains that queue on its next frame,
//! where opening a document is an ordinary thing to do — and where a document
//! that wants a password can ask for one, which is not a conversation to have
//! from inside an event handler.

use std::sync::Mutex;

/// Paths Finder has handed over and the application has not yet taken.
static PENDING: Mutex<Vec<std::path::PathBuf>> = Mutex::new(Vec::new());

/// Take whatever Finder has handed over since this was last called.
pub fn taken() -> Vec<std::path::PathBuf> {
    match PENDING.lock() {
        Ok(mut queue) => std::mem::take(&mut *queue),
        // Poisoned by a panic in the handler. An empty list is the right
        // answer: the alternative is refusing to open documents for the rest of
        // the session.
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn listen() {}

#[cfg(not(target_os = "macos"))]
pub fn watch_for_delegate() {}

#[cfg(target_os = "macos")]
pub use mac::{listen, watch_for_delegate};

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Sel};
    use objc2::{class, msg_send, sel};

    use super::PENDING;

    /// Start answering Finder's open-document events.
    ///
    /// Registered against `NSAppleEventManager` rather than by replacing the
    /// application delegate, because winit installs a delegate of its own and
    /// two of them cannot both be the delegate. An event handler is additive.
    /// Give winit's application delegate an `application:openFile:`.
    ///
    /// **The timing is the whole problem.** When a document causes the launch,
    /// AppKit calls `application:openFile:` *before*
    /// `applicationDidFinishLaunching:` — so anything registered from inside
    /// the window-creation callback is already too late. Measured: the method
    /// was added successfully and never called, on every cold launch.
    ///
    /// So it is added to the delegate's **class**, looked up by the name winit
    /// registers it under, from a thread started at the top of `main` — before
    /// winit has created the delegate at all.
    ///
    /// **Nothing here touches AppKit.** `objc_getClass` and `class_addMethod`
    /// are runtime operations with no main-thread requirement; asking
    /// `NSApplication` for its delegate instead crashed the program outright,
    /// in an AppKit assertion that the main event queue was not the current
    /// one. A background thread may read the runtime; it may not wake AppKit.
    pub fn watch_for_delegate() {
        std::thread::spawn(|| {
            // A second is far longer than the window between winit registering
            // the class and AppKit dispatching the document. Giving up matters:
            // a thread that never stops is worse than a document that did not
            // open.
            for _ in 0..10_000 {
                if teach_the_delegate() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
        });
    }

    pub fn listen() {
        // `kCoreEventClass` / `kAEOpenDocuments`, which are four-character
        // codes: 'aevt' and 'odoc'.
        const AEVT: u32 = u32::from_be_bytes(*b"aevt");
        const ODOC: u32 = u32::from_be_bytes(*b"odoc");

        unsafe {
            let manager: *mut AnyObject =
                msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
            if manager.is_null() {
                return;
            }
            let Some(handler) = handler_object() else {
                return;
            };
            let _: () = msg_send![
                manager,
                setEventHandler: &*handler,
                andSelector: sel!(handleOpen:withReply:),
                forEventClass: AEVT,
                andEventID: ODOC,
            ];
            // Deliberately leaked: it has to outlive this call and live as long
            // as the application does, and there is exactly one.
            std::mem::forget(handler);
        }

        teach_the_delegate();
    }

    /// Give the application's delegate an `application:openFile:` of its own.
    ///
    /// **The event handler above is not enough on its own.** AppKit installs
    /// its own handler for the same event while the application finishes
    /// launching, and the document that *caused* the launch is dispatched
    /// before any code of ours gets to run — measured, our handler never fired
    /// for it while firing reliably for every open afterwards. AppKit's handler
    /// asks the delegate; a delegate with no answer is why macOS says the
    /// application "cannot open files in the PDF document format".
    ///
    /// So the method is added to whatever class the delegate happens to be —
    /// winit's, here — rather than by replacing the delegate, which would take
    /// the window's own events away with it.
    fn teach_the_delegate() -> bool {
        use objc2::runtime::AnyClass;

        // By name, not by asking the application — see `watch_for_delegate`.
        let Some(class) = AnyClass::get(c"WinitApplicationDelegate") else {
            return false;
        };
        unsafe {
            let implementation: objc2::runtime::Imp = std::mem::transmute::<
                extern "C" fn(&AnyObject, Sel, *mut AnyObject, *mut AnyObject) -> bool,
                unsafe extern "C-unwind" fn(),
            >(open_file);
            // `class_addMethod` leaves an existing implementation alone, so
            // this can never take a method away from winit.
            let _added = objc2::ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                sel!(application:openFile:),
                implementation,
                c"B@:@@".as_ptr(),
            );
        }
        true
    }

    /// The delegate method AppKit calls with the launch document.
    extern "C" fn open_file(
        _this: &AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
        path: *mut AnyObject,
    ) -> bool {
        if path.is_null() {
            return false;
        }
        let spelled = unsafe {
            let utf8: *const std::os::raw::c_char = msg_send![path, UTF8String];
            if utf8.is_null() {
                return false;
            }
            std::ffi::CStr::from_ptr(utf8).to_str().map(str::to_owned)
        };
        let Ok(spelled) = spelled else { return false };
        if let Ok(mut queue) = PENDING.lock() {
            queue.push(std::path::PathBuf::from(spelled));
        }
        true
    }

    /// An object with one method on it, for the manager to call back into.
    fn handler_object() -> Option<Retained<AnyObject>> {
        use objc2::runtime::ClassBuilder;

        let class = match AnyClass::get(c"PagifyOpenHandler") {
            // Registered already, which happens if this is called twice.
            Some(existing) => existing,
            None => {
                let superclass = class!(NSObject);
                let mut builder = ClassBuilder::new(c"PagifyOpenHandler", superclass)?;
                unsafe {
                    builder.add_method(
                        sel!(handleOpen:withReply:),
                        handle_open as extern "C" fn(_, _, _, _),
                    );
                }
                builder.register()
            }
        };
        unsafe {
            let object: *mut AnyObject = msg_send![class, new];
            Retained::from_raw(object)
        }
    }

    /// The callback itself. Pulls the file list out of the event and queues it.
    extern "C" fn handle_open(
        _this: &AnyObject,
        _cmd: Sel,
        event: *mut AnyObject,
        _reply: *mut AnyObject,
    ) {
        let paths = unsafe { paths_in(event) };
        if paths.is_empty() {
            return;
        }
        if let Ok(mut queue) = PENDING.lock() {
            queue.extend(paths);
        }
    }

    /// The paths an open-document event carries.
    ///
    /// They arrive as a descriptor list of URLs, one-based as AppleScript
    /// counts. Anything that is not a readable URL is skipped rather than
    /// guessed at.
    unsafe fn paths_in(event: *mut AnyObject) -> Vec<std::path::PathBuf> {
        const DIRECT: u32 = u32::from_be_bytes(*b"----");
        let mut out = Vec::new();
        if event.is_null() {
            return out;
        }

        let list: *mut AnyObject = unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT] };
        if list.is_null() {
            return out;
        }
        let count: i32 = unsafe { msg_send![list, numberOfItems] };
        for index in 1..=count {
            let item: *mut AnyObject = unsafe { msg_send![list, descriptorAtIndex: index] };
            if item.is_null() {
                continue;
            }
            let text: *mut AnyObject = unsafe { msg_send![item, stringValue] };
            if text.is_null() {
                continue;
            }
            let url: *mut AnyObject =
                unsafe { msg_send![class!(NSURL), URLWithString: text] };
            if url.is_null() {
                continue;
            }
            let path: *mut AnyObject = unsafe { msg_send![url, path] };
            if path.is_null() {
                continue;
            }
            let utf8: *const std::os::raw::c_char = unsafe { msg_send![path, UTF8String] };
            if utf8.is_null() {
                continue;
            }
            let spelled = unsafe { std::ffi::CStr::from_ptr(utf8) };
            if let Ok(spelled) = spelled.to_str() {
                out.push(std::path::PathBuf::from(spelled));
            }
        }
        out
    }
}
