package com.hsilighting.pagify.data

import android.content.Context
import android.util.Log
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.ContactGroup
import com.hsilighting.pagify.core.NativeBridge
import com.hsilighting.pagify.core.contactFromCardJson
import com.hsilighting.pagify.core.toCardJson
import com.hsilighting.pagify.data.db.ContactsDatabase
import com.hsilighting.pagify.data.db.GroupRow
import com.hsilighting.pagify.data.db.MembershipRow
import com.hsilighting.pagify.data.db.toContact
import com.hsilighting.pagify.data.db.toGroup
import com.hsilighting.pagify.data.db.toRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/**
 * The contacts read off business cards, and the groups they are filed in.
 *
 * Backed by Room rather than the JSON file the recents and settings use, for the
 * reasons on [ContactsDatabase]. The store's job is to keep the app's own types
 * on the outside: nothing above this line knows what a `ContactRow` is.
 *
 * Unlike the recents list, **this is the user's data.** A contact exists nowhere
 * else once the card is in a drawer, so failures are logged loudly rather than
 * swallowed.
 */
class ContactStore(context: Context) {

    private val dao = ContactsDatabase.get(context).contacts()

    val contacts: Flow<List<Contact>> = dao.contacts().map { rows -> rows.map { it.toContact() } }

    val groups: Flow<List<ContactGroup>> = dao.groups().map { rows -> rows.map { it.toGroup() } }

    /** Which contacts are in which groups, as contact id to group ids. */
    val memberships: Flow<Map<Long, List<Long>>> = dao.memberships().map { rows ->
        rows.groupBy({ it.contactId }, { it.groupId })
    }

    // ------------------------------------------------------------ contacts --

    suspend fun save(contact: Contact) {
        withContext(Dispatchers.IO) { dao.save(contact.toRow()) }
    }

    /**
     * Save a card and file it in one step.
     *
     * The group comes from the import session rather than being asked per card:
     * after an event somebody imports forty of these, and answering the same
     * question forty times is what makes a feature go unused. A null group is
     * ordinary and means ungrouped.
     */
    suspend fun save(contact: Contact, intoGroup: Long?) {
        withContext(Dispatchers.IO) {
            dao.save(contact.toRow())
            if (intoGroup != null) {
                dao.addToGroup(
                    MembershipRow(
                        contactId = contact.id,
                        groupId = intoGroup,
                        addedAt = System.currentTimeMillis(),
                    ),
                )
            }
        }
    }

    suspend fun delete(id: Long) {
        withContext(Dispatchers.IO) { dao.deleteContact(id) }
    }

    // -------------------------------------------------------------- groups --

    suspend fun saveGroup(group: ContactGroup) {
        withContext(Dispatchers.IO) { dao.saveGroup(group.toRow()) }
    }

    /**
     * Delete a group, and **not** its contacts.
     *
     * They lose this membership, keep every other one, and become ungrouped if it
     * was their last. A container that deletes its contents when it goes is the
     * natural implementation and a data-loss bug; there is a test.
     */
    suspend fun deleteGroup(id: Long) {
        withContext(Dispatchers.IO) { dao.deleteGroup(id) }
    }

    suspend fun addToGroup(contactId: Long, groupId: Long) {
        withContext(Dispatchers.IO) {
            dao.addToGroup(MembershipRow(contactId, groupId, System.currentTimeMillis()))
        }
    }

    suspend fun removeFromGroup(contactId: Long, groupId: Long) {
        withContext(Dispatchers.IO) { dao.removeFromGroup(contactId, groupId) }
    }

    suspend fun mergeGroups(from: Long, into: Long) {
        withContext(Dispatchers.IO) { dao.merge(from, into) }
    }

    suspend fun contactsIn(groupId: Long): List<Contact> =
        withContext(Dispatchers.IO) { dao.contactsInGroup(groupId).map { it.toContact() } }

    suspend fun ungrouped(): List<Contact> =
        withContext(Dispatchers.IO) { dao.ungrouped().map { it.toContact() } }

    // -------------------------------------------------------------- export --

    /**
     * Turn contacts into a vCard, and record that it happened.
     *
     * The stamping is not a side effect of exporting — as far as this feature is
     * concerned it *is* the export. One timestamp is taken and used everywhere:
     * in each stored contact, in each vCard's `REV`, and on the group if this was
     * a group export. They were sent together, so nothing may disagree about
     * when.
     */
    suspend fun exportToVCard(chosen: List<Contact>, group: Long? = null): String {
        if (chosen.isEmpty()) return ""

        val now = System.currentTimeMillis()
        val stamp = rfc3339(now)

        return withContext(Dispatchers.IO) {
            val json = "[" + chosen.joinToString(",") { it.toCardJson().toString() } + "]"
            val vcard = NativeBridge.contactsToVCard(json, stamp)

            dao.markExported(chosen.map { it.id }, now)
            group?.let { dao.markGroupExported(it, now) }
            vcard
        }
    }

    /** Everyone in a group, as one file. */
    suspend fun exportGroup(group: ContactGroup): String =
        exportToVCard(contactsIn(group.id), group.id)

    /**
     * Read a QR payload as a contact, or null when it is not one.
     *
     * Null is the ordinary answer: most QR codes on business cards hold a web
     * address. Telling the two apart is what lets the caller fall through to
     * reading the card by eye instead of saving a blank contact.
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

    private companion object {
        const val TAG = "ContactStore"
    }
}

/**
 * The timestamp format vCard's `REV` expects: RFC 3339, in UTC.
 *
 * UTC rather than local time because the file travels. A contact exported in
 * Dubai and opened in London should not appear to have been sent four hours in
 * the future.
 */
fun rfc3339(millis: Long): String =
    SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US)
        .apply { timeZone = TimeZone.getTimeZone("UTC") }
        .format(Date(millis))
