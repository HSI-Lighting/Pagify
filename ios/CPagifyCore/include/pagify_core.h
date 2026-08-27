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

char *pagify_recognise_stroke(const char *points_json);

// -- cache --------------------------------------------------------------------

int32_t pagify_set_cache_budget_bytes(int64_t handle, int64_t budget_bytes);
int32_t pagify_clear_cache(int64_t handle);
char *pagify_get_cache_stats_json(int64_t handle);
/// The onTrimMemory twin: >= 80 closes documents, lower only drops cached rasters.
void pagify_on_trim_memory(int32_t level);

#ifdef __cplusplus
}
#endif

#endif // PAGIFY_CORE_H
