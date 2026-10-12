//! Native Windows printing: listing printers, the driver's own settings
//! dialog, the paper a printer reports, and the GDI calls that send a page's
//! own rendered bitmap to the printer.
//!
//! Windows-only — see `pagify_app/Cargo.toml`'s own
//! `cfg(target_os = "windows")` dependency block. The print window
//! (`print_window.rs`) is the one caller, gated the same way; no other
//! platform has a printing feature yet.
//!
//! **Why not the OS Print dialog any more.** On Windows 11 22H2+ the OS swaps
//! `PrintDlgExW` for its modern dialog, whose preview pane an app only fills by
//! supplying a WinRT print source — which an egui window cannot. So the print
//! window is Pagify's own, and this module is what it needs from the OS:
//! `DocumentPropertiesW` still shows the *driver's* dialog (duplex, tray,
//! staple, paper size) on request.
//!
//! **A raster image, not a document of vector commands.** Everything this
//! app already knows how to do is render a page to an RGBA bitmap
//! (`Session::render_page_rotated`) — so printing reuses exactly that,
//! requested at the size the page will land on the sheet
//! (`pagify_shell::print_layout::place`, the same answer the preview draws
//! from) rather than screen resolution. A printer DC commonly reports 300–1200
//! DPI against the ~96–150 DPI a screen preview uses; blitting a
//! screen-resolution bitmap stretched up to fill a full page would look
//! visibly soft, so each page is re-rendered specifically for the print job.

use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, GetDeviceCaps, SetBrushOrgEx, SetStretchBltMode, StretchDIBits, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DEVMODEW, DEVMODE_FIELD_FLAGS, DIB_RGB_COLORS, DMCOLLATE_FALSE,
    DMCOLLATE_TRUE, DMORIENT_LANDSCAPE, DMORIENT_PORTRAIT, DM_COLLATE, DM_COPIES, DM_ORIENTATION, HALFTONE, HDC,
    HORZRES, LOGPIXELSX, LOGPIXELSY, PHYSICALHEIGHT, PHYSICALOFFSETX, PHYSICALOFFSETY, PHYSICALWIDTH, SRCCOPY,
    VERTRES,
};
use windows::Win32::Graphics::Printing::{
    ClosePrinter, DocumentPropertiesW, EnumPrintersW, GetDefaultPrinterW, OpenPrinterW, PRINTER_ENUM_CONNECTIONS,
    PRINTER_ENUM_LOCAL, PRINTER_HANDLE, PRINTER_INFO_4W,
};
// The actual GDI print-job calls — see this crate dependency's own doc
// comment in Cargo.toml for why these, despite taking a GDI `HDC`, live
// under `Storage::Xps` in this version of the `windows` crate rather than
// `Graphics::Gdi`.
use windows::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};

use pagify_shell::print_layout::{self, Options, Paper};
use pagify_shell::Session;

// `DocumentPropertiesW` `fMode` bits and results (wingdi.h / winuser.h).
const DM_OUT_BUFFER: u32 = 2;
const DM_IN_PROMPT: u32 = 4;
const DM_IN_BUFFER: u32 = 8;
const IDOK: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterInfo {
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// An open printer handle, closed on every way out — `DocumentPropertiesW`
/// needs one and a leaked handle keeps the spooler's printer object pinned.
struct OpenPrinter(PRINTER_HANDLE);

impl OpenPrinter {
    fn new(name: &str) -> Result<Self, String> {
        let w = wide(name);
        let mut handle = PRINTER_HANDLE::default();
        unsafe { OpenPrinterW(PCWSTR(w.as_ptr()), &mut handle, None) }
            .map_err(|e| format!("couldn't open the printer \"{name}\": {e}"))?;
        Ok(OpenPrinter(handle))
    }
}

impl Drop for OpenPrinter {
    fn drop(&mut self) {
        unsafe {
            let _ = ClosePrinter(self.0);
        }
    }
}

/// Every printer this person can print to, the default flagged.
///
/// Level 4 (name and attributes only) on purpose: it is answered from the
/// spooler's own list and never wakes a network device, which a deeper level
/// does — a copier that is off would otherwise stall the window opening.
pub fn list_printers() -> Result<Vec<PrinterInfo>, String> {
    let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
    let mut needed = 0u32;
    let mut returned = 0u32;
    // The first call only asks how big the answer is; it "fails" with
    // ERROR_INSUFFICIENT_BUFFER by design.
    let _ = unsafe { EnumPrintersW(flags, PCWSTR::null(), 4, None, &mut needed, &mut returned) };
    if needed == 0 {
        return Ok(Vec::new());
    }
    // A `u64` store: `PRINTER_INFO_4W` holds pointers, so it must be 8-aligned,
    // which a `Vec<u8>` does not promise.
    let mut store = vec![0u64; (needed as usize).div_ceil(8)];
    let bytes = unsafe { std::slice::from_raw_parts_mut(store.as_mut_ptr() as *mut u8, needed as usize) };
    unsafe { EnumPrintersW(flags, PCWSTR::null(), 4, Some(bytes), &mut needed, &mut returned) }
        .map_err(|e| format!("couldn't list the printers: {e}"))?;

    let default = default_printer_name();
    let infos = unsafe { std::slice::from_raw_parts(store.as_ptr() as *const PRINTER_INFO_4W, returned as usize) };
    let mut out: Vec<PrinterInfo> = infos
        .iter()
        .filter_map(|info| unsafe { info.pPrinterName.to_string().ok() })
        .map(|name| PrinterInfo { is_default: Some(&name) == default.as_ref(), name })
        .collect();
    out.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out.dedup_by(|a, b| a.name == b.name);
    Ok(out)
}

fn default_printer_name() -> Option<String> {
    let mut len = 0u32;
    let _ = unsafe { GetDefaultPrinterW(None, &mut len) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    if !unsafe { GetDefaultPrinterW(Some(PWSTR(buf.as_mut_ptr())), &mut len) }.as_bool() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// One printer and the settings chosen for it: the whole `DEVMODEW`, including
/// the driver's own private bytes after it (`dmDriverExtra`), which hold the
/// duplex / tray / staple choices and must travel with it untouched.
#[derive(Clone)]
pub struct PrinterSetup {
    pub name: String,
    // `u64`s so the `DEVMODEW` inside is aligned however the allocator feels.
    buf: Vec<u64>,
}

impl PrinterSetup {
    /// The driver's own defaults for this printer.
    pub fn new(name: &str) -> Result<Self, String> {
        let printer = OpenPrinter::new(name)?;
        let device = wide(name);
        let size = unsafe { DocumentPropertiesW(None, printer.0, PCWSTR(device.as_ptr()), None, None, 0) };
        if size <= 0 {
            return Err(format!("the printer \"{name}\" would not say what its settings are."));
        }
        let mut setup = PrinterSetup { name: name.to_string(), buf: vec![0u64; (size as usize).div_ceil(8)] };
        let got = unsafe {
            DocumentPropertiesW(None, printer.0, PCWSTR(device.as_ptr()), Some(setup.devmode_mut()), None, DM_OUT_BUFFER)
        };
        if got != IDOK {
            return Err(format!("the printer \"{name}\" would not give its default settings."));
        }
        Ok(setup)
    }

    fn devmode(&self) -> *const DEVMODEW {
        self.buf.as_ptr() as *const DEVMODEW
    }
    fn devmode_mut(&mut self) -> *mut DEVMODEW {
        self.buf.as_mut_ptr() as *mut DEVMODEW
    }

    pub fn orientation(&self) -> Orientation {
        let o = unsafe { (*self.devmode()).Anonymous1.Anonymous1.dmOrientation };
        if o as u32 == DMORIENT_LANDSCAPE {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        }
    }

    pub fn copies(&self) -> u32 {
        unsafe { (*self.devmode()).Anonymous1.Anonymous1.dmCopies }.max(1) as u32
    }

    /// Set orientation, copies and collation, and let the driver validate and
    /// merge them (`DM_IN_BUFFER | DM_OUT_BUFFER`). A field is only honoured
    /// when its `dmFields` bit is set, which is why each is set here — a
    /// driver that ignores one does so silently, so read the result back.
    pub fn apply(&mut self, orientation: Orientation, copies: u32, collate: bool) -> Result<(), String> {
        unsafe {
            let dm = &mut *self.devmode_mut();
            dm.dmFields = DEVMODE_FIELD_FLAGS(dm.dmFields.0 | DM_ORIENTATION.0 | DM_COPIES.0 | DM_COLLATE.0);
            dm.Anonymous1.Anonymous1.dmOrientation = match orientation {
                Orientation::Portrait => DMORIENT_PORTRAIT,
                Orientation::Landscape => DMORIENT_LANDSCAPE,
            } as i16;
            dm.Anonymous1.Anonymous1.dmCopies = copies.clamp(1, 9999) as i16;
            dm.dmCollate = if collate { DMCOLLATE_TRUE } else { DMCOLLATE_FALSE };
        }
        self.merge(None)?;
        Ok(())
    }

    /// The driver's own settings dialog (duplex, tray, staple, paper size).
    /// `Ok(true)` if the person pressed OK, `Ok(false)` if they cancelled.
    pub fn show_properties(&mut self, owner: HWND) -> Result<bool, String> {
        self.merge(Some(owner))
    }

    /// Run the settings through `DocumentPropertiesW`, input and output in
    /// separate buffers (the same one for both is not something its contract
    /// promises). With an owner, the driver's dialog is shown.
    fn merge(&mut self, owner: Option<HWND>) -> Result<bool, String> {
        let printer = OpenPrinter::new(&self.name)?;
        let device = wide(&self.name);
        let input = self.buf.clone();
        let mode = DM_IN_BUFFER | DM_OUT_BUFFER | if owner.is_some() { DM_IN_PROMPT } else { 0 };
        let got = unsafe {
            DocumentPropertiesW(
                owner,
                printer.0,
                PCWSTR(device.as_ptr()),
                Some(self.devmode_mut()),
                Some(input.as_ptr() as *const DEVMODEW),
                mode,
            )
        };
        if got < 0 {
            // A failed merge may have left the output half-written: put it back.
            self.buf = input;
            return Err(format!("the printer \"{}\" refused those settings.", self.name));
        }
        if got != IDOK && owner.is_none() {
            return Err(format!("the printer \"{}\" refused those settings.", self.name));
        }
        Ok(got == IDOK)
    }
}

/// A printer DC for these settings. Drawing nothing to it is free — it is how
/// the sheet's size is learned without printing.
unsafe fn open_dc(setup: &PrinterSetup) -> Result<HDC, String> {
    let device = wide(&setup.name);
    // The driver name is ignored for printers (the documented advice), so null.
    let hdc = CreateDCW(PCWSTR::null(), PCWSTR(device.as_ptr()), PCWSTR::null(), Some(setup.devmode()));
    if hdc.is_invalid() {
        return Err(format!("couldn't open the printer \"{}\".", setup.name));
    }
    Ok(hdc)
}

unsafe fn paper_of_dc(hdc: HDC) -> Paper {
    let caps = |index| GetDeviceCaps(Some(hdc), index).max(0) as u32;
    Paper {
        physical: (caps(PHYSICALWIDTH), caps(PHYSICALHEIGHT)),
        offset: (caps(PHYSICALOFFSETX), caps(PHYSICALOFFSETY)),
        printable: (caps(HORZRES), caps(VERTRES)),
        dpi: (GetDeviceCaps(Some(hdc), LOGPIXELSX).max(1) as f32, GetDeviceCaps(Some(hdc), LOGPIXELSY).max(1) as f32),
    }
}

/// The sheet these settings print on. Asked again only when the printer,
/// orientation or driver settings change — a network printer can be slow to
/// answer.
pub fn paper(setup: &PrinterSetup) -> Result<Paper, String> {
    unsafe {
        let hdc = open_dc(setup)?;
        let paper = paper_of_dc(hdc);
        let _ = DeleteDC(hdc);
        if paper.printable.0 == 0 || paper.printable.1 == 0 {
            return Err(format!("the printer \"{}\" reports no printable area.", setup.name));
        }
        Ok(paper)
    }
}

/// The scale to render a page at for the printer: the wanted `scale`
/// (pixels per point), or less when that would ask the engine for more than
/// its safety ceiling (`MAX_DIMENSION_PX` / `MAX_PIXELS`) allows.
///
/// **Reported from use: "couldn't render page 1 for printing: requested render
/// is too large: 9743x6888 px"** — an A3 sheet at the printer's 600 dpi. The
/// clamp already existed and landed *exactly* on the ceiling, but the engine
/// rounds the pixel counts up, so a request that is exactly at the limit comes
/// out a few pixels over it and is refused. Hence the margin: a little under
/// the ceiling, so what is asked for is what is allowed.
fn render_scale(scale: f32, width_pt: f32, height_pt: f32) -> f32 {
    let max_dim = pdf_core::render::bitmap::MAX_DIMENSION_PX as f32 * 0.99;
    let max_pixels = pdf_core::render::bitmap::MAX_PIXELS as f32 * 0.97;
    scale
        .min(max_dim / width_pt)
        .min(max_dim / height_pt)
        .min((max_pixels / (width_pt * height_pt)).sqrt())
}

/// `PageRaster`'s own RGBA8, top-left origin, converted to the top-down BGRA
/// `StretchDIBits` wants under `BI_RGB` — a channel swap only. Both are
/// already top-down, so no row flip is needed; `biHeight` negative in the
/// `BITMAPINFOHEADER` is what tells GDI that, not a reordering of the bytes.
fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        out.push(px[2]);
        out.push(px[1]);
        out.push(px[0]);
        out.push(px[3]);
    }
    out
}

/// Send `pages` (zero-based) to the printer in one job, each placed by
/// [`print_layout::place`]. `output` names a file for a print-to-file driver
/// ("Microsoft Print to PDF") so it does not ask — what the end-to-end test
/// uses; a real job passes `None`.
pub fn print(
    setup: &PrinterSetup,
    session: &Session,
    title: &str,
    pages: &[usize],
    options: &Options,
    output: Option<&Path>,
) -> Result<(), String> {
    if pages.is_empty() {
        return Err("no pages matched what was asked for.".into());
    }
    unsafe {
        let hdc = open_dc(setup)?;
        let outcome = print_to_dc(hdc, session, title, pages, options, output);
        let _ = DeleteDC(hdc);
        outcome
    }
}

unsafe fn print_to_dc(
    hdc: HDC,
    session: &Session,
    title: &str,
    pages: &[usize],
    options: &Options,
    output: Option<&Path>,
) -> Result<(), String> {
    let paper = paper_of_dc(hdc);

    let doc_name = wide(title);
    let output_name = output.map(|p| wide(&p.to_string_lossy()));
    let docinfo = DOCINFOW {
        cbSize: std::mem::size_of::<DOCINFOW>() as i32,
        lpszDocName: PCWSTR(doc_name.as_ptr()),
        lpszOutput: output_name.as_ref().map_or(PCWSTR::null(), |w| PCWSTR(w.as_ptr())),
        lpszDatatype: PCWSTR::null(),
        fwType: 0,
    };
    if StartDocW(hdc, &docinfo) <= 0 {
        return Err("the printer refused to start the job.".into());
    }

    for &page in pages {
        let size = session.page_size(page).map_err(|e| {
            let _ = AbortDoc(hdc);
            format!("couldn't read page {}: {e}", page + 1)
        })?;
        let placement = print_layout::place(&paper, (size.width_pt, size.height_pt), options);
        if placement.dest.w <= 0 || placement.dest.h <= 0 {
            let _ = AbortDoc(hdc);
            return Err(format!("page {} has no size to print.", page + 1));
        }

        // Render at the size the page lands at, so the blit below is close to
        // 1:1 — clamped to what the engine allows (an A0 drawing at 300+ dpi
        // asks for more pixels than its safety ceiling, and prints a little
        // softer rather than not at all; **reported from use** on a real
        // 20-page architectural submission).
        let turned = matches!(placement.rotation, pdf_core::document::Rotation::Clockwise90);
        let (page_w, page_h) = if turned { (size.height_pt, size.width_pt) } else { (size.width_pt, size.height_pt) };
        let scale = render_scale(placement.dest.w as f32 / page_w, page_w, page_h);
        let raster = session.render_page_rotated(page, scale, placement.rotation).map_err(|e| {
            let _ = AbortDoc(hdc);
            format!("couldn't render page {} for printing: {e}", page + 1)
        })?;

        let bgra = rgba_to_bgra(&raster.pixels);
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: raster.width as i32,
                // Negative: a top-down DIB, matching the RGBA buffer's own
                // row order — no manual row flip needed.
                biHeight: -(raster.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };

        if StartPage(hdc) <= 0 {
            let _ = AbortDoc(hdc);
            return Err(format!("the printer refused to start page {}.", page + 1));
        }
        // HALFTONE gives a better-quality downscale than the GDI default —
        // but it requires this exact companion call right after, or output
        // shows visible tiling artefacts (a documented, easy-to-miss Windows
        // requirement).
        SetStretchBltMode(hdc, HALFTONE);
        let _ = SetBrushOrgEx(hdc, 0, 0, None);

        let blitted = StretchDIBits(
            hdc,
            placement.dest.x,
            placement.dest.y,
            placement.dest.w,
            placement.dest.h,
            0,
            0,
            raster.width as i32,
            raster.height as i32,
            Some(bgra.as_ptr() as *const _),
            &bitmap_info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        if blitted <= 0 {
            let _ = AbortDoc(hdc);
            return Err(format!("couldn't draw page {} onto the printer.", page + 1));
        }
        if EndPage(hdc) <= 0 {
            let _ = AbortDoc(hdc);
            return Err(format!("the printer refused to finish page {}.", page + 1));
        }
    }

    if EndDoc(hdc) <= 0 {
        return Err("the printer refused to finish the job.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sheet that is allowed is asked for at a size the engine accepts,
    /// whatever the printer's resolution: the rendered pixel counts (rounded
    /// up, as the engine does) stay inside both of its limits.
    #[test]
    fn a_big_sheet_is_rendered_at_a_size_the_engine_accepts() {
        let max_dim = pdf_core::render::bitmap::MAX_DIMENSION_PX as u64;
        let max_pixels = pdf_core::render::bitmap::MAX_PIXELS;
        // A3 landscape (the reported one), A1, A0, a long roll, and a small
        // page, at 300, 600 and 1200 dpi.
        for (w, h) in [(1190.55, 841.89), (1683.78, 2383.94), (2383.94, 3370.39), (300.0, 20_000.0), (595.0, 842.0)] {
            for dpi in [300.0f32, 600.0, 1200.0] {
                let scale = render_scale(dpi / 72.0, w, h);
                let (pw, ph) = ((w * scale).ceil() as u64, (h * scale).ceil() as u64);
                assert!(pw <= max_dim && ph <= max_dim, "{w}x{h} at {dpi} dpi asks for {pw}x{ph}");
                assert!(pw * ph <= max_pixels, "{w}x{h} at {dpi} dpi asks for {pw}x{ph} = {} pixels", pw * ph);
            }
        }
        // A page that fits is still printed at the printer's own resolution.
        assert_eq!(render_scale(600.0 / 72.0, 595.0, 842.0), 600.0 / 72.0);
    }

    #[test]
    fn rgba_to_bgra_swaps_colour_channels_and_keeps_alpha() {
        let rgba = [10u8, 20, 30, 40, 50, 60, 70, 80];
        assert_eq!(rgba_to_bgra(&rgba), vec![30, 20, 10, 40, 70, 60, 50, 80]);
    }

    /// The default printer is listed first and flagged — the window opens on it.
    #[test]
    fn the_default_printer_is_listed_first_and_flagged() {
        let printers = list_printers().expect("the spooler answers");
        if printers.is_empty() {
            return; // a machine with no printers has nothing to check
        }
        assert_eq!(printers.iter().filter(|p| p.is_default).count(), 1, "{printers:?}");
        assert!(printers[0].is_default, "{printers:?}");
        let mut names: Vec<_> = printers.iter().map(|p| &p.name).collect();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "a printer listed twice");
    }

    /// Orientation and copies must actually *take*: a wrong `dmFields` bit
    /// makes a driver ignore a setting without a word, so read it back.
    #[test]
    fn orientation_and_copies_survive_the_drivers_own_merge() {
        let Some(printer) = list_printers().ok().and_then(|p| p.into_iter().find(|p| p.name == "Microsoft Print to PDF")) else {
            return; // the PDF printer is a Windows component, but skip rather than fail without it
        };
        let mut setup = PrinterSetup::new(&printer.name).expect("default settings");
        setup.apply(Orientation::Landscape, 1, false).expect("merge landscape");
        assert_eq!(setup.orientation(), Orientation::Landscape);
        let landscape = paper(&setup).expect("paper");
        setup.apply(Orientation::Portrait, 1, false).expect("merge portrait");
        assert_eq!(setup.orientation(), Orientation::Portrait);
        let portrait = paper(&setup).expect("paper");
        // The sheet itself turned, not just a flag.
        assert!(landscape.printable.0 > landscape.printable.1, "{landscape:?}");
        assert!(portrait.printable.0 < portrait.printable.1, "{portrait:?}");
        assert!(portrait.printable.0 <= portrait.physical.0 && portrait.printable.1 <= portrait.physical.1);
    }
}
