package com.hsilighting.pagify.data

import android.content.Context
import android.util.Log
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.NativeBridge
import com.hsilighting.pagify.core.contactFromCardJson
import com.hsilighting.pagify.core.contactsFromStoreJson
import com.hsilighting.pagify.core.toCardJson
import com.hsilighting.pagify.core.toStoreJson
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * The contacts read off business cards, kept across launches.
 *
 * A JSON file in the app's own storage, following `RecentDocumentsStore` rather
 * than introducing a database. The plan originally assumed Room was already a
 * dependency; it is not, and for a list somebody scrolls rather than queries a
 * schema and a migration path would be machinery for a problem this does not yet
 * have. If searching the raw text ever becomes slow, that is the moment to
 * revisit it — and searching is done in memory here, so the change would be
 * contained.
 *
 * Unlike the recents list, **this is the user's data.** A contact exists nowhere
 * else once the card is in a drawer. So failures here are logged loudly and a
 * write that fails does not silently drop the contact from memory.
 */
class ContactStore(context: Context) {

    private val file = File(context.filesDir, FILE_NAME)

    private val _contacts = MutableStateFlow<List<Contact>>(emptyList())
    val contacts: StateFlow<List<Contact>> = _contacts.asStateFlow()

    suspend fun load() {
        val stored = withContext(Dispatchers.IO) {
            runCatching {
                if (file.exists()) contactsFromStoreJson(file.readText()) else emptyList()
            }
                .onFailure { Log.e(TAG, "could not read the contacts", it) }
                .getOrDefault(emptyList())
        }
        _contacts.value = stored.sortedByDescending { it.capturedAt }
    }

    /** Add a contact, or replace one with the same id. */
    suspend fun save(contact: Contact) {
        val without = _contacts.value.filterNot { it.id == contact.id }
        write((without + contact).sortedByDescending { it.capturedAt })
    }

    suspend fun delete(id: Long) {
        write(_contacts.value.filterNot { it.id == id })
    }

    /**
     * Turn contacts into a vCard, and record that it happened.
     *
     * The stamping is not a side effect of exporting — it *is* the export, as far
     * as this feature is concerned. `exportedAt` goes into the stored contact and
     * the same instant goes into the file as `REV`, so the two never disagree
     * about when the contact left.
     */
    suspend fun exportToVCard(chosen: List<Contact>): String {
        if (chosen.isEmpty()) return ""

        val now = System.currentTimeMillis()
        val stamp = rfc3339(now)

        val vcard = withContext(Dispatchers.IO) {
            val json = "[" + chosen.joinToString(",") { it.toCardJson().toString() } + "]"
            NativeBridge.contactsToVCard(json, stamp)
        }

        val exported = chosen.map { it.id }.toSet()
        write(
            _contacts.value.map { contact ->
                if (contact.id in exported) {
                    contact.copy(exportedAt = now, exportCount = contact.exportCount + 1)
                } else {
                    contact
                }
            },
        )
        return vcard
    }

    /**
     * Read a QR payload as a contact, or null when it is not one.
     *
     * Null is the ordinary answer, not a failure: most QR codes on business cards
     * hold a web address. Telling the two apart is what lets the caller fall
     * through to reading the card by eye instead of saving a blank contact.
     */
    suspend fun contactFromQr(payload: String): Contact? = withContext(Dispatchers.IO) {
        runCatching {
            NativeBridge.contactFromVCard(payload)?.let {
                contactFromCardJson(it, System.currentTimeMillis())
            }
        }
            .onFailure { Log.w(TAG, "could not read the QR payload as a contact", it) }
            .getOrNull()
    }

    private suspend fun write(contacts: List<Contact>) {
        // Memory first, so the UI reflects the change even if the disk write
        // fails — losing the file is recoverable next launch, but losing the
        // contact somebody just scanned in front of them is not.
        _contacts.value = contacts
        withContext(Dispatchers.IO) {
            runCatching { file.writeText(contacts.toStoreJson()) }
                .onFailure { Log.e(TAG, "could not write the contacts", it) }
        }
    }

    private companion object {
        const val FILE_NAME = "contacts.json"
        const val TAG = "ContactStore"
    }
}

/**
 * The timestamp format vCard's `REV` expects: RFC 3339, in UTC.
 *
 * UTC rather than local time because the file travels. A contact exported in
 * Dubai and opened in London should not appear to have been exported four hours
 * in the future.
 */
fun rfc3339(millis: Long): String =
    SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US)
        .apply { timeZone = TimeZone.getTimeZone("UTC") }
        .format(Date(millis))
