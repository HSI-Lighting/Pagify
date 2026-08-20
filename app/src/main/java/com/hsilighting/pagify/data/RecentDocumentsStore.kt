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
class RecentDocumentsStore(context: Context) {

    private val file = File(context.filesDir, FILE_NAME)

    private val _documents = MutableStateFlow<List<RecentDocument>>(emptyList())
    val documents: StateFlow<List<RecentDocument>> = _documents.asStateFlow()

    /**
     * Read the file into memory.
     *
     * Called once, off the main thread. Until it completes the library shows an
     * empty list, which is the truthful thing to show while the answer is unknown
     * and takes a few milliseconds to stop being true.
     */
    suspend fun load() {
        val stored = withContext(Dispatchers.IO) {
            runCatching { if (file.exists()) recentsFromJson(file.readText()) else emptyList() }
                .onFailure { Log.w(TAG, "could not read the library", it) }
                .getOrDefault(emptyList())
        }
        _documents.value = stored
    }

    /** Record a document as just opened, and write the list back. */
    suspend fun remember(document: RecentDocument) {
        write(promoteRecent(_documents.value, document))
    }

    /**
     * Drop a document from the list.
     *
     * For a file that has been moved, deleted, or whose grant a reboot took away:
     * the row is a promise the app can no longer keep, and leaving it there means
     * offering the same failure every time.
     */
    suspend fun forget(uri: String) {
        write(forgetRecent(_documents.value, uri))
    }

    /** Forget every document. The files themselves are not touched. */
    suspend fun clear() = write(emptyList())

    private suspend fun write(documents: List<RecentDocument>) {
        // In memory first. The screen should not wait on a disk write to show a
        // document it has already opened.
        _documents.value = documents

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
