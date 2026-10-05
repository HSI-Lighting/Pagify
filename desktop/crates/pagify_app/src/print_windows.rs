//! Native Windows printing: the standard Print dialog, and the GDI calls
//! that send a page's own rendered bitmap through it to the printer the
//! user chose.
//!
//! Windows-only — see `pagify_app/Cargo.toml`'s own
//! `cfg(target_os = "windows")` dependency block. `main.rs`'s
//! `print_current_document` is the one caller, gated the same way; no other
//! platform has a printing feature yet.
//!
//! **A raster image, not a document of vector commands.** Everything this
//! app already knows how to do is render a page to an RGBA bitmap
//! (`Session::render_page_rotated`) — so printing reuses exactly that,
//! requested at the *printer's own reported DPI* rather than screen
//! resolution. A printer DC commonly reports 300–1200 DPI against the
//! ~96–150 DPI a screen preview uses; blitting a screen-resolution bitmap
//! stretched up to fill a full page would look visibly soft, so each page
//! is re-rendered specifically for the print job instead of reusing
//! whatever the screen already had cached.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GlobalFree, HWND};
use windows::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, GetDeviceCaps, SetBrushOrgEx, SetStretchBltMode, StretchDIBits, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DEVMODEW, DIB_RGB_COLORS, HALFTONE, HORZRES, LOGPIXELSX, LOGPIXELSY,
    SRCCOPY, VERTRES,
};
// The actual GDI print-job calls — see this crate dependency's own doc
// comment in Cargo.toml for why these, despite taking a GDI `HDC`, live
// under `Storage::Xps` in this version of the `windows` crate rather than
// `Graphics::Gdi`.
use windows::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::UI::Controls::Dialogs::{
    PrintDlgExW, DEVNAMES, PD_ALLPAGES, PD_CURRENTPAGE, PD_PAGENUMS, PD_RESULT_PRINT,
    PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGEXW, PRINTPAGERANGE, START_PAGE_GENERAL,
};

use pagify_shell::Session;

/// Which zero-based page indices to print, resolved from the dialog's own
/// result flags and (for a range) the page-range list it filled in.
///
/// Pure — no dialog, no handles — so this, the one piece of actual *logic*
/// in this module, is the one piece that can be unit tested directly rather
/// than only by hand against a real printer.
fn resolve_print_pages(
    flags: u32,
    current_page: usize,
    ranges: &[(u32, u32)],
    page_count: usize,
) -> Vec<usize> {
    if page_count == 0 {
        return Vec::new();
    }
    if flags & PD_CURRENTPAGE.0 != 0 {
        return vec![current_page.min(page_count - 1)];
    }
    if flags & PD_PAGENUMS.0 != 0 && !ranges.is_empty() {
        let mut pages: Vec<usize> = Vec::new();
        for &(from, to) in ranges {
            // The dialog's own page numbers are one-based, clamped to what
            // this document actually has — `nMinPage`/`nMaxPage` already
            // constrain what a person can type in, but a defensive clamp
            // here costs nothing and a wrong one would print the wrong page.
            let from = from.saturating_sub(1) as usize;
            let to = to.saturating_sub(1) as usize;
            if from >= page_count {
                continue;
            }
            for p in from..=to.min(page_count - 1) {
                if !pages.contains(&p) {
                    pages.push(p);
                }
            }
        }
        pages.sort_unstable();
        return pages;
    }
    // `PD_ALLPAGES`, or nothing recognisable set — default to everything
    // rather than printing nothing, the same way a cancelled-but-not-really
    // dialog state should never silently produce an empty job.
    let _ = PD_ALLPAGES;
    (0..page_count).collect()
}

/// The scale to render a page at for the printer: the printer's own `scale`
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

/// Read a NUL-terminated UTF-16 string out of a `DEVNAMES`/`DEVMODEW`-style
/// block at a given `u16` character offset from its own start.
///
/// **Reported from use**: `CreateDCW` failed outright ("couldn't open the
/// chosen printer") on every real attempt. The copied slice stopped at
/// `len` — the last *real* character, one short of the NUL terminator at
/// index `len` — so the `Vec` handed to `PCWSTR` was never actually
/// NUL-terminated itself. `PCWSTR` reads until it finds one regardless, so
/// it kept reading straight past this `Vec`'s own allocation into whatever
/// memory happened to follow it, handing `CreateDCW` a driver/device/port
/// name corrupted by trailing garbage that could never match a real
/// printer.
unsafe fn wide_str_at(base: *const u16, offset_chars: usize) -> Vec<u16> {
    let start = base.add(offset_chars);
    let mut len = 0usize;
    while *start.add(len) != 0 {
        len += 1;
    }
    // `len + 1`: the NUL terminator itself, included rather than trimmed.
    std::slice::from_raw_parts(start, len + 1).to_vec()
}

/// Show the native Print dialog, and — if the user actually prints rather
/// than cancelling or only changing settings — send the chosen page range
/// to the chosen printer.
pub fn print_document(
    owner: HWND,
    session: &Session,
    page_count: usize,
    current_page: usize,
    title: &str,
) -> Result<(), String> {
    if page_count == 0 {
        return Err("nothing to print.".into());
    }

    const MAX_RANGES: u32 = 16;
    let mut page_ranges = vec![PRINTPAGERANGE { nFromPage: 0, nToPage: 0 }; MAX_RANGES as usize];

    let mut pdex = PRINTDLGEXW {
        lStructSize: std::mem::size_of::<PRINTDLGEXW>() as u32,
        hwndOwner: owner,
        nMinPage: 1,
        nMaxPage: page_count as u32,
        nMaxPageRanges: MAX_RANGES,
        lpPageRanges: page_ranges.as_mut_ptr(),
        nCopies: 1,
        Flags: PD_ALLPAGES | PD_USEDEVMODECOPIESANDCOLLATE,
        nStartPage: START_PAGE_GENERAL,
        ..Default::default()
    };

    unsafe { PrintDlgExW(&mut pdex) }.map_err(|e| format!("couldn't open the print dialog: {e}"))?;

    if pdex.dwResultAction != PD_RESULT_PRINT {
        // Cancelled, or only "Apply" was pressed (settings changed, no
        // print requested) — neither is an error.
        return Ok(());
    }

    let flags = pdex.Flags.0;
    let ranges: Vec<(u32, u32)> =
        page_ranges.iter().take(pdex.nPageRanges as usize).map(|r| (r.nFromPage, r.nToPage)).collect();
    let pages = resolve_print_pages(flags, current_page, &ranges, page_count);
    if pages.is_empty() {
        return Err("no pages matched what was asked for.".into());
    }

    let result = unsafe { print_pages(&pdex, session, title, &pages) };

    unsafe {
        if !pdex.hDevMode.is_invalid() {
            let _ = GlobalFree(Some(pdex.hDevMode));
        }
        if !pdex.hDevNames.is_invalid() {
            let _ = GlobalFree(Some(pdex.hDevNames));
        }
    }

    result
}

/// The part that actually touches the printer: a device context built from
/// the dialog's own chosen printer/settings, one `StartDoc`/`EndDoc` job,
/// one `StartPage`/`StretchDIBits`/`EndPage` per page.
unsafe fn print_pages(
    pdex: &PRINTDLGEXW,
    session: &Session,
    title: &str,
    pages: &[usize],
) -> Result<(), String> {
    let devmode_lock = GlobalLock(pdex.hDevMode) as *const DEVMODEW;
    if devmode_lock.is_null() {
        return Err("couldn't read the chosen printer's settings.".into());
    }
    let devnames_lock = GlobalLock(pdex.hDevNames) as *const u16;
    if devnames_lock.is_null() {
        GlobalUnlock(pdex.hDevMode).ok();
        return Err("couldn't read the chosen printer's name.".into());
    }
    let devnames = devnames_lock as *const DEVNAMES;

    let driver = wide_str_at(devnames_lock, (*devnames).wDriverOffset as usize);
    let device = wide_str_at(devnames_lock, (*devnames).wDeviceOffset as usize);
    let port = wide_str_at(devnames_lock, (*devnames).wOutputOffset as usize);

    let hdc = CreateDCW(
        PCWSTR(driver.as_ptr()),
        PCWSTR(device.as_ptr()),
        PCWSTR(port.as_ptr()),
        Some(devmode_lock),
    );
    GlobalUnlock(pdex.hDevMode).ok();
    GlobalUnlock(pdex.hDevNames).ok();

    if hdc.is_invalid() {
        return Err("couldn't open the chosen printer.".into());
    }

    let outcome = print_to_dc(hdc, session, title, pages);
    let _ = DeleteDC(hdc);
    outcome
}

unsafe fn print_to_dc(hdc: windows::Win32::Graphics::Gdi::HDC, session: &Session, title: &str, pages: &[usize]) -> Result<(), String> {
    let dpi_x = GetDeviceCaps(Some(hdc), LOGPIXELSX).max(1) as f32;
    let dpi_y = GetDeviceCaps(Some(hdc), LOGPIXELSY).max(1) as f32;
    // `HORZRES`/`VERTRES` are already the *printable* area in device
    // pixels, and (0, 0) on a printer DC is already that area's own
    // top-left — not the full physical paper's, which is what
    // `PHYSICALWIDTH`/`PHYSICALOFFSETX` and friends describe instead.
    // Fitting within these two needs neither.
    let printable_w = GetDeviceCaps(Some(hdc), HORZRES).max(1);
    let printable_h = GetDeviceCaps(Some(hdc), VERTRES).max(1);

    let mut doc_name: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let docinfo = DOCINFOW {
        cbSize: std::mem::size_of::<DOCINFOW>() as i32,
        lpszDocName: PCWSTR(doc_name.as_mut_ptr()),
        lpszOutput: PCWSTR::null(),
        lpszDatatype: PCWSTR::null(),
        fwType: 0,
    };
    if StartDocW(hdc, &docinfo) <= 0 {
        return Err("the printer refused to start the job.".into());
    }

    for &page in pages {
        let scale = dpi_x.max(dpi_y) / 72.0;
        // The printer's own full native DPI, regardless of page size — fine
        // for a normal sheet, but an oversized one (an A1/A0 architectural
        // drawing) at 300+ DPI can ask for more pixels than the engine's own
        // safety ceiling allows (`MAX_DIMENSION_PX`/`MAX_PIXELS`, guarding
        // against an OOM-sized buffer). That request was made once, at this
        // one scale, with no fallback — so the whole page silently failed to
        // print instead of printing a little softer. **Reported from use**
        // on a real 20-page architectural submission. Clamped here the same
        // way the on-screen path already clamps before ever asking pdf_core
        // to rasterise, rather than finding out from a render error.
        let scale = match session.page_size(page) {
            Ok(size) => render_scale(scale, size.width_pt, size.height_pt),
            Err(_) => scale,
        };
        let raster = session
            .render_page_rotated(page, scale, pdf_core::document::Rotation::None)
            .map_err(|e| {
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
        // HALFTONE gives a better-quality downscale than the GDI default
        // when the printer-DPI render still needs to be fit to the
        // printable area — but it requires this exact companion call right
        // after, or output shows visible tiling artefacts (a documented,
        // easy-to-miss Windows requirement).
        SetStretchBltMode(hdc, HALFTONE);
        let _ = SetBrushOrgEx(hdc, 0, 0, None);

        // Fit the printer-DPI render to the printable area, preserving
        // aspect ratio — the two should already match closely (the page
        // was rendered at this printer's own DPI specifically) but a
        // rounding difference between the page's own point size and the
        // printer's reported resolution is still possible.
        let scale_to_fit = (printable_w as f32 / raster.width as f32).min(printable_h as f32 / raster.height as f32);
        let dest_w = (raster.width as f32 * scale_to_fit) as i32;
        let dest_h = (raster.height as f32 * scale_to_fit) as i32;

        let blitted = StretchDIBits(
            hdc,
            0,
            0,
            dest_w,
            dest_h,
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

    /// **Reported from use**: every real print attempt failed at `CreateDCW`
    /// ("couldn't open the chosen printer"). Traced to this function handing
    /// back a slice that stopped one character short of the NUL terminator
    /// it had just found — `PCWSTR` reads until it sees one regardless, so
    /// it kept reading past the end of the returned `Vec`'s own allocation
    /// into whatever memory happened to follow, corrupting the driver and
    /// device name `CreateDCW` was given.
    #[test]
    fn wide_str_at_includes_its_own_nul_terminator() {
        // "AB" followed by its NUL, followed by a sentinel value that must
        // never be reached — if it is, the function read short and this
        // assertion still accidentally passes, so the real proof is the
        // *length*, not just the content.
        let buf: [u16; 4] = [b'A' as u16, b'B' as u16, 0, 0xFFFF];
        let found = unsafe { wide_str_at(buf.as_ptr(), 0) };
        assert_eq!(found, vec![b'A' as u16, b'B' as u16, 0], "must include the NUL, not stop one short of it");
        assert_eq!(*found.last().unwrap(), 0, "the last element must be the terminator itself");
    }

    #[test]
    fn wide_str_at_reads_from_the_given_offset() {
        let buf: [u16; 5] = [0xDEAD, b'X' as u16, b'Y' as u16, 0, 0xFFFF];
        let found = unsafe { wide_str_at(buf.as_ptr(), 1) };
        assert_eq!(found, vec![b'X' as u16, b'Y' as u16, 0]);
    }

    #[test]
    fn all_pages_flag_prints_everything() {
        assert_eq!(resolve_print_pages(PD_ALLPAGES.0, 0, &[], 5), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn current_page_flag_prints_just_that_one() {
        assert_eq!(resolve_print_pages(PD_CURRENTPAGE.0, 2, &[], 5), vec![2]);
    }

    #[test]
    fn current_page_flag_clamps_to_the_last_real_page() {
        assert_eq!(resolve_print_pages(PD_CURRENTPAGE.0, 99, &[], 5), vec![4]);
    }

    #[test]
    fn a_single_range_is_one_based_in_zero_based_out() {
        assert_eq!(resolve_print_pages(PD_PAGENUMS.0, 0, &[(2, 4)], 10), vec![1, 2, 3]);
    }

    #[test]
    fn several_ranges_are_merged_and_deduplicated() {
        assert_eq!(resolve_print_pages(PD_PAGENUMS.0, 0, &[(1, 3), (2, 5)], 10), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_range_past_the_end_is_clamped_rather_than_producing_nothing() {
        assert_eq!(resolve_print_pages(PD_PAGENUMS.0, 0, &[(3, 999)], 5), vec![2, 3, 4]);
    }

    #[test]
    fn pagenums_flag_with_no_ranges_falls_back_to_everything() {
        // Defensive: a dialog result that claims a page-number selection
        // but supplies no actual range should not silently print nothing.
        assert_eq!(resolve_print_pages(PD_PAGENUMS.0, 0, &[], 3), vec![0, 1, 2]);
    }

    #[test]
    fn zero_pages_prints_nothing() {
        assert_eq!(resolve_print_pages(PD_ALLPAGES.0, 0, &[], 0), Vec::<usize>::new());
    }

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
}
