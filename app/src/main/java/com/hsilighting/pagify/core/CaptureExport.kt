package com.hsilighting.pagify.core

import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import androidx.core.content.FileProvider
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Getting a capture out of the app: to a file, to another app, to the clipboard.
 *
 * Kept apart from [CaptureRequest] and friends so the decisions there stay
 * testable off-device; everything in here needs a `Context` and a filesystem.
 */
object CaptureExport {

    /** Where cached captures live, relative to `cacheDir`. Mirrors `capture_paths.xml`. */
    private const val CACHE_SUBDIRECTORY = "captures"

    /** Album the gallery groups saved captures under. */
    private const val ALBUM = "Pagify"

    fun timestamp(): String =
        SimpleDateFormat("yyyy-MM-dd HH-mm-ss", Locale.US).format(Date())

    /**
     * Write a capture into the cache and return a URI another app can read.
     *
     * A `content://` URI from our own [FileProvider], never a `file://` path:
     * handing out a file path has thrown `FileUriExposedException` since API 24,
     * and the grant is what lets the receiving app read one file of ours without
     * being given the rest.
     *
     * Old captures are swept first. They are the app's own cache, they are large,
     * and nothing refers to them once the share sheet has closed.
     */
    fun cache(context: Context, bytes: ByteArray, fileName: String): Uri {
        val directory = File(context.cacheDir, CACHE_SUBDIRECTORY).apply { mkdirs() }
        sweep(directory)

        val file = File(directory, fileName)
        file.writeBytes(bytes)

        return FileProvider.getUriForFile(context, "${context.packageName}.captures", file)
    }

    /**
     * Save a capture to the device's gallery.
     *
     * On API 29+ this needs no permission at all: the row is inserted into
     * `MediaStore` and the bytes are written through the URI it hands back, so the
     * app never touches a path outside its own sandbox. Below 29 there is no
     * scoped storage and `WRITE_EXTERNAL_STORAGE` is required, which the manifest
     * declares with `maxSdkVersion="28"` — the caller checks for it there and
     * nowhere else.
     *
     * `IS_PENDING` keeps the row invisible until the bytes are written, so the
     * gallery cannot show a half-written image if this is interrupted.
     */
    fun saveToGallery(
        context: Context,
        bytes: ByteArray,
        fileName: String,
        format: CaptureFormat,
    ): Uri {
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, fileName)
            put(MediaStore.Images.Media.MIME_TYPE, format.mimeType)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                put(
                    MediaStore.Images.Media.RELATIVE_PATH,
                    "${Environment.DIRECTORY_PICTURES}/$ALBUM",
                )
                put(MediaStore.Images.Media.IS_PENDING, 1)
            }
        }

        val resolver = context.contentResolver
        val uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
            ?: error("the gallery refused a new image")

        try {
            resolver.openOutputStream(uri)?.use { it.write(bytes) }
                ?: error("the gallery gave no stream to write to")
        } catch (failure: Throwable) {
            // A row with no bytes behind it is worse than no row: it shows in the
            // gallery as a permanently broken thumbnail.
            resolver.delete(uri, null, null)
            throw failure
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            resolver.update(
                uri,
                ContentValues().apply { put(MediaStore.Images.Media.IS_PENDING, 0) },
                null,
                null,
            )
        }

        return uri
    }

    /** True when saving to the gallery is possible without asking for anything. */
    fun galleryNeedsPermission(): Boolean = Build.VERSION.SDK_INT < Build.VERSION_CODES.Q

    /**
     * A share sheet for one capture.
     *
     * `FLAG_GRANT_READ_URI_PERMISSION` is what makes the URI readable by whichever
     * app the user picks; without it every receiver gets a security exception and
     * the share silently does nothing.
     */
    fun shareIntent(uri: Uri, format: CaptureFormat): Intent =
        Intent.createChooser(
            Intent(Intent.ACTION_SEND).apply {
                type = format.mimeType
                putExtra(Intent.EXTRA_STREAM, uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            },
            null,
        ).apply { addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION) }

    /**
     * Put a capture on the clipboard.
     *
     * `ClipData.newUri` rather than a plain text URI: it carries the MIME type, so
     * a paste target knows it is being handed an image and can inline it instead
     * of pasting a path as text.
     */
    fun copyToClipboard(context: Context, uri: Uri) {
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newUri(context.contentResolver, "Pagify capture", uri))
    }

    /**
     * Delete cached captures older than a day.
     *
     * Bounded rather than unbounded: these are full-resolution images, a session
     * of capturing can be tens of megabytes, and once a share sheet has closed
     * nothing points at the file any more. A day is long enough that an app the
     * user shared to can still resolve the URI it was given.
     */
    private fun sweep(directory: File) {
        val cutoff = System.currentTimeMillis() - MILLIS_PER_DAY
        directory.listFiles()?.forEach { file ->
            if (file.isFile && file.lastModified() < cutoff) file.delete()
        }
    }

    private const val MILLIS_PER_DAY = 24L * 60 * 60 * 1000
}
