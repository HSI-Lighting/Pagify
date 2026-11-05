package com.hsilighting.pagify.data

import android.content.Context
import android.util.Log
import com.hsilighting.pagify.core.RecentDocument
import com.hsilighting.pagify.core.forgetRecent
import com.hsilighting.pagify.core.promoteRecent
import com.hsilighting.pagify.core.recentsFromJson
import com.hsilighting.pagify.core.toRecentsJson
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File

/**
 * The library's list of documents, kept across launches.
 *
 * A JSON file in the app's own storage rather than a database: it is one flat list
 * of at most a few dozen rows, read once at startup and rewritten when a document
 * is opened. A schema, a migration path and a query language would all be
 * machinery for a problem this does not have.
 *
 * The list is exposed as a flow so the library screen redraws when a document is
 * opened from anywhere — the picker, a share from another app, or a row on the
 * screen itself.
 *
 * Nothing here is the user's data. The documents are wherever they always were;
 * this is a memory of having seen them, and losing it costs a list, not a file.
 * That is why every failure below is swallowed rather than raised.
 */
class RecentDocumentsStore(private val file: File) {

    constructor(context: Context) : this(File(context.filesDir, FILE_NAME))

    private val _documents = MutableStateFlow<List<RecentDocument>>(emptyList())
    val documents: StateFlow<List<RecentDocument>> = _documents.asStateFlow()

    // Every change rewrites the whole file from the list in memory, so a change
    // made before the file has been read writes out only itself. A cold start
    // from another app's "Open with" did exactly that: the drawing was
    // remembered while `load()` was still running, and the library became that
    // one row (seen twice on the phone, 2026-10-12). So every change reads the
    // file first if nothing has yet, under one lock that also keeps two writes
    // from reaching the disk in the wrong order.
    private val lock = Mutex()
    private var loaded = false

    /**
     * Read the file into memory.
     *
     * Called once at startup. Until it completes the library shows an empty
     * list, which is the truthful thing to show while the answer is unknown and
     * takes a few milliseconds to stop being true.
     */
    suspend fun load() = lock.withLock { loadOnce() }

    /** Record a document as just opened, and write the list back. */
    suspend fun remember(document: RecentDocument) = lock.withLock {
        loadOnce()
        write(promoteRecent(_documents.value, document))
    }

    /**
     * Drop a document from the list.
     *
     * For a file that has been moved, deleted, or whose grant a reboot took away:
     * the row is a promise the app can no longer keep, and leaving it there means
     * offering the same failure every time.
     */
    suspend fun forget(uri: String) = lock.withLock {
        loadOnce()
        write(forgetRecent(_documents.value, uri))
    }

    /** Forget every document. The files themselves are not touched. */
    suspend fun clear() = lock.withLock { write(emptyList()) }

    private suspend fun loadOnce() {
        if (loaded) return
        _documents.value = withContext(Dispatchers.IO) {
            runCatching { if (file.exists()) recentsFromJson(file.readText()) else emptyList() }
                .onFailure { Log.w(TAG, "could not read the library", it) }
                .getOrDefault(emptyList())
        }
        loaded = true
    }

    private suspend fun write(documents: List<RecentDocument>) {
        // In memory first. The screen should not wait on a disk write to show a
        // document it has already opened.
        _documents.value = documents
        loaded = true

        withContext(Dispatchers.IO) {
            runCatching { file.writeText(documents.toRecentsJson()) }
                .onFailure { Log.w(TAG, "could not write the library", it) }
        }
    }

    private companion object {
        const val FILE_NAME = "recent-documents.json"
        const val TAG = "RecentDocuments"
    }
}
