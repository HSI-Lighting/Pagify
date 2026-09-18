// The C ABI exported by `rust/pdf_core/src/ffi`. This header is the contract;
// if a signature here disagrees with the Rust side the mismatch is silent until
// it corrupts memory, so change both together.
//
// Ownership, in one place:
//   - `char *` returns are owned by the caller. Release with pagify_string_free.
//     NULL means failure; the reason is on pagify_last_error_message().
//   - PagifyBuffer returns are owned by the caller. Release with
//     pagify_buffer_free. A NULL `data` means failure.
//   - `fd` arguments are ADOPTED: the callee closes them on every path,
//     including the failing ones. Never close one yourself after passing it.
//   - Anything returning int32_t returns 0 for success and -1 for failure
//     unless its comment says otherwise.

#ifndef PAGIFY_CORE_H
#define PAGIFY_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define PAGIFY_INVALID_HANDLE ((int64_t)-1)
#define PAGIFY_OK ((int32_t)0)
#define PAGIFY_ERROR ((int32_t)-1)

/// Byte order of a 4-bytes-per-pixel buffer, as passed to pagify_render_page_into.
typedef enum {
    PagifyPixelOrderRGBA = 0,
    PagifyPixelOrderBGRA = 1,
} PagifyPixelOrder;

typedef struct {
    uint8_t *data;
    size_t len;
    size_t cap;
} PagifyBuffer;

// -- errors and memory --------------------------------------------------------

/// The failure message for the most recent call *on this thread*, or NULL if it
/// succeeded. Caller frees.
char *pagify_last_error_message(void);
void pagify_string_free(char *value);
void pagify_buffer_free(PagifyBuffer buffer);

// -- lifecycle ----------------------------------------------------------------

void pagify_init(void);

/// Point the engine at a PDFium build. Must be called before the first open;
/// returns PAGIFY_ERROR if PDFium is already bound.
int32_t pagify_set_pdfium_library_path(const char *path);

char *pagify_version(void);

int64_t pagify_open_document(const char *path, const char *password);
/// Adopts `fd` — see the ownership note at the top.
int64_t pagify_open_document_fd(int32_t fd, const char *password);
bool pagify_close_document(int64_t handle);
int32_t pagify_open_document_count(void);

// -- reading ------------------------------------------------------------------

/// Page count, or -1 on failure.
int32_t pagify_get_page_count(int64_t handle);
/// Writes [width, height] in points into `out_size`, which needs room for two.
int32_t pagify_get_page_size(int64_t handle, int32_t page_index, float *out_size);
/// Quarter turns, or -1 on failure.
int32_t pagify_get_page_rotation(int64_t handle, int32_t page_index);

char *pagify_get_metadata_json(int64_t handle);
char *pagify_get_page_text(int64_t handle, int32_t page_index);
char *pagify_get_text_segments_json(int64_t handle, int32_t page_index);
char *pagify_get_page_characters_json(int64_t handle, int32_t page_index);
char *pagify_get_annotations_json(int64_t handle, int32_t page_index);
char *pagify_get_text_marks_json(int64_t handle, int32_t page_index);

// -- rendering ----------------------------------------------------------------

/// Render into a caller-supplied buffer. The buffer's own dimensions decide the
/// render size; `zoom` only identifies the cache entry.
///
/// Returns 1 if the pixels came from cache, 0 if they were rendered, -1 on
/// failure. `stride` must be at least `width * 4`.
int32_t pagify_render_page_into(int64_t handle,
                                int32_t page_index,
                                float zoom,
                                int32_t rotation_quarter_turns,
                                uint8_t *pixels,
                                uint32_t width,
                                uint32_t height,
                                size_t stride,
                                int32_t pixel_order);

/// Returns 1 if work was done, 0 if already cached, -1 on failure.
int32_t pagify_prefetch_page(int64_t handle, int32_t page_index, float zoom,
                             int32_t rotation_quarter_turns);

// -- editing ------------------------------------------------------------------

/// The whole editing model. `command_json` is a serde-tagged `Command`; the
/// return is the resulting edit state as JSON. A new operation needs no new
/// entry point here.
char *pagify_execute_command_json(int64_t handle, const char *command_json);
char *pagify_undo_edit(int64_t handle);
char *pagify_redo_edit(int64_t handle);
char *pagify_get_edit_state_json(int64_t handle);

/// Adopts `fd`. **`fd` must not be the file this document was opened from** —
/// PDFium reads lazily for the document's whole life, so that truncates the
/// input mid-save. Write to a scratch file and copy over.
int32_t pagify_save_to_fd(int64_t handle, int32_t fd, bool incremental);

/// Write chosen pages out as their own PDF. `indices_json` is a JSON array of
/// page indices **in the order they should appear**. Adopts `fd`.
int32_t pagify_export_pages_to_fd(int64_t handle, const char *indices_json, int32_t fd);

/// Bring another open document's pages into this one, after `at`. Returns the
/// resulting edit state as JSON; caller frees.
char *pagify_import_pages(int64_t handle, int64_t source_handle,
                          const char *indices_json, int32_t at);

/// Adopts `fd`. `fill` is packed 0xAARRGGBB, or 0 for no fill.
int32_t pagify_create_blank_document(int32_t fd, int32_t pages, float width_pt,
                                     float height_pt, int32_t fill, int32_t ruling);

// -- text and fonts -----------------------------------------------------------

/// The bytes are copied; free them as soon as this returns.
int32_t pagify_register_font(const char *name, const uint8_t *data, size_t len);
bool pagify_font_covers(const char *name, const char *text);
/// {"rtl":bool,"glyphs":[{"id","from","to","advance","dx","dy"}]}
char *pagify_shape_text_json(const char *name, const char *text);

// -- capture ------------------------------------------------------------------

PagifyBuffer pagify_capture_region(int64_t handle, int32_t page_index, float left,
                                   float top, float right, float bottom, float scale,
                                   const char *format, int32_t quality,
                                   const char *markup_json);

PagifyBuffer pagify_capture_viewport(int64_t handle, const char *tiles_json, float width,
                                     float height, float scale, int32_t background,
                                     const char *format, int32_t quality,
                                     const char *markup_json, const char *mask_json);

/// Burn marks into an RGBA8 buffer with no document behind it — a capture
/// taken from the STEP model or DXF/DWG drawing viewers, where
/// pagify_capture_region/pagify_capture_viewport do not apply since there is
/// no page to re-render a region from. Marks are in capture-local points,
/// top-left origin, scaled onto the buffer by `scale` exactly as a page
/// capture's are. Pure pixels in, marks drawn on, pixels out: no handle, no
/// lock, safe to call on any thread the moment a capture picture exists.
/// Same buffer contract as pagify_render_page_into. markup_json may be NULL.
int32_t pagify_composite_markup_into(uint32_t width, uint32_t height, uint8_t *pixels,
                                     size_t stride, float scale, const char *markup_json);

char *pagify_recognise_stroke(const char *points_json);

// -- cache --------------------------------------------------------------------

int32_t pagify_set_cache_budget_bytes(int64_t handle, int64_t budget_bytes);
int32_t pagify_clear_cache(int64_t handle);
char *pagify_get_cache_stats_json(int64_t handle);
/// The onTrimMemory twin: >= 80 closes documents, lower only drops cached rasters.
void pagify_on_trim_memory(int32_t level);

// -- 3D model (STEP) ------------------------------------------------------------
//
// One flattened mesh plus a trackball camera — a STEP assembly's parts are
// placed in world space during tessellation and never exposed as a tree (see
// rust/pdf_core/src/step/assembly.rs). orbit/pan/zoom/fit only move the
// camera; call pagify_render_model_into again afterwards for a fresh frame.

/// Opens by path; NULL/-1 sentinel conventions match pagify_open_document.
/// Failure is most often the audit's own refusal sentence (too many faces, no
/// solid found) rather than a generic message — see pagify_last_error_message().
int64_t pagify_open_model(const char *path);
bool pagify_close_model(int64_t handle);
char *pagify_model_summary_json(int64_t handle);

/// across/down are fractions of the view, not pixels — matches Android's own
/// orbit/pan exactly, including using the view *width* for both axes so a
/// non-square viewport doesn't skew one axis against the other.
int32_t pagify_orbit_model(int64_t handle, float across, float down);
int32_t pagify_pan_model(int64_t handle, float across, float down);
/// by > 1 moves closer, matching a pinch's apart/lastApart ratio.
int32_t pagify_zoom_model(int64_t handle, float by);
int32_t pagify_fit_model(int64_t handle);

/// Same contract as pagify_render_page_into: stride must be at least
/// width * 4, and the buffer must be `stride * height` bytes, writable for
/// the call. Always RGBA8 (the software rasteriser has no separate byte-order
/// mode) — convert if the destination needs BGRA.
int32_t pagify_render_model_into(int64_t handle, uint32_t width, uint32_t height,
                                 uint8_t *pixels, size_t stride);

// -- drawings (DXF/DWG) -----------------------------------------------------------
//
// DWG and DXF are not two engines: dwg.rs converts into the exact same
// `Drawing` type dxf.rs parses directly, so everything past open — render,
// pan, zoom, measure, layers — is one code path regardless of which file
// came in. Hatching needs no entry point of its own: it is expanded into
// strokes inside the render call, scaled to the current zoom.

/// font_name may be NULL; if given, it must already be registered with
/// pagify_register_font. Drawings carry no usable font of their own
/// (SHX/Windows fonts never travel with CAD files) — best-effort, a missing
/// or unreadable font does not fail the open.
int64_t pagify_open_drawing(const char *path, const char *font_name);
bool pagify_close_drawing(int64_t handle);
char *pagify_drawing_summary_json(int64_t handle);
/// Layer list as JSON: name, visibility, colour.
char *pagify_drawing_layers_json(int64_t handle);
int32_t pagify_show_drawing_layer(int64_t handle, int64_t at, bool visible);

/// across/down are fractions of the view (a one-finger drag); width/height
/// are the current viewport size, since the pan distance in drawing-space
/// depends on the current zoom scale.
int32_t pagify_pan_drawing(int64_t handle, float across, float down,
                          uint32_t width, uint32_t height);
/// at_x/at_y are a screen-space anchor (pixels, e.g. a pinch's midpoint) that
/// stays fixed on screen; by > 1 moves closer.
int32_t pagify_zoom_drawing(int64_t handle, float by, float at_x, float at_y,
                           uint32_t width, uint32_t height);
int32_t pagify_fit_drawing(int64_t handle, uint32_t width, uint32_t height);

/// Places a measure point at a screen position, snapped to nearby geometry,
/// and returns the running measurement as JSON — what was snapped to, and
/// once two points are placed, the distance in the drawing's own units
/// (never converted to metric: half of these files never declare real-world
/// units). A third tap starts over rather than chaining.
char *pagify_measure_drawing_at(int64_t handle, float at_x, float at_y,
                                uint32_t width, uint32_t height);
int32_t pagify_clear_drawing_measure(int64_t handle);

/// Same contract as pagify_render_page_into/pagify_render_model_into. `by` is
/// how much larger this bitmap is than the view fit/pan/zoom were last called
/// with — 1.0 for an ordinary live frame, greater for a capture rendered
/// above screen resolution. Not optional the way it looks: a sheet's scale is
/// pixels per drawing unit and does not derive from the bitmap it fills, so
/// passed 1.0 for a bitmap that is actually larger this shows more of the
/// drawing rather than the same part of it more sharply — a crop, not a
/// capture of what was on screen.
int32_t pagify_render_drawing_into(int64_t handle, uint32_t width, uint32_t height,
                                   float by, uint8_t *pixels, size_t stride);

// -- contacts / vCard ----------------------------------------------------------

/// Split a photograph's recognised text into cards and parse each one.
/// `segments_json` is a JSON array of {left, top, right, bottom, text} in the
/// photograph's own pixel space, unscaled. Returns a JSON array of
/// BusinessCard (possibly empty), or NULL on failure.
char *pagify_parse_photographed_card(const char *segments_json);

/// Render one card (a JSON-encoded BusinessCard) as a single-VCARD vCard 3.0
/// file. `exported_at` is an RFC 3339 UTC instant, written into REV. NULL on
/// failure — see pagify_last_error_message().
char *pagify_vcard(const char *card_json, const char *exported_at);

/// As pagify_vcard, for a JSON array of cards: one file, concatenated VCARD
/// blocks, all sharing the same REV.
char *pagify_vcards(const char *cards_json, const char *exported_at);

/// Read a scanned QR payload back into a card (JSON-encoded BusinessCard), or
/// NULL if `text` is not a vCard at all — an ordinary outcome, not a failure:
/// pagify_last_error_message() stays unset. NULL with a message set means
/// `text` itself was NULL, a real argument error.
char *pagify_vcard_parse(const char *text);

#ifdef __cplusplus
}
#endif

#endif // PAGIFY_CORE_H
