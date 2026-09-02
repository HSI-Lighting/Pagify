package com.hsilighting.pagify

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.ContactGroup
import com.hsilighting.pagify.data.ContactStore
import com.hsilighting.pagify.data.db.ContactsDatabase
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Filing, through the store the app actually calls.
 *
 * The schema had tests and the DAO had tests; the layer between them had none,
 * and neither did the flows the screen reads. That gap is where "adding to a
 * group does nothing" would live and go unnoticed — every write can succeed and
 * the app still show nothing, because what the screen renders is not the write
 * but the flow that should follow it.
 *
 * So these assert on the **flows**, not on a direct read back. A write that
 * lands in the database but never reaches a collector is a bug the user sees and
 * a read-back test does not.
 */
@RunWith(AndroidJUnit4::class)
class ContactStoreTest {

    private lateinit var database: ContactsDatabase
    private lateinit var store: ContactStore

    @Before
    fun open() {
        database = Room.inMemoryDatabaseBuilder(
            InstrumentationRegistry.getInstrumentation().targetContext,
            ContactsDatabase::class.java,
        ).build()
        store = ContactStore(database)
    }

    @After
    fun close() = database.close()

    private fun contact(id: Long, name: String) = Contact(id = id, name = name)

    private fun group(id: Long, name: String) = ContactGroup(id = id, name = name)

    /** Wait for a flow to say what we are waiting for, rather than sleeping. */
    private suspend fun <T> awaiting(flow: kotlinx.coroutines.flow.Flow<T>, until: (T) -> Boolean): T =
        withTimeout(5_000) { flow.first(until) }

    @Test
    fun savingIntoAGroupShowsUpInTheMemberships() = runBlocking {
        store.saveGroup(group(10, "Light + Building 2026"))
        store.save(contact(1, "Jane Okafor"), intoGroup = 10)

        val memberships = awaiting(store.memberships) { it[1L]?.contains(10L) == true }
        assertEquals(listOf(10L), memberships[1L])
    }

    /** The route the UI takes when filing a contact that already exists. */
    @Test
    fun addingAnExistingContactToAGroupReachesTheFlow() = runBlocking {
        store.save(contact(1, "Jane Okafor"), intoGroup = null)
        store.saveGroup(group(10, "Suppliers"))
        awaiting(store.groups) { it.any { group -> group.id == 10L } }

        store.addToGroup(contactId = 1, groupId = 10)

        val memberships = awaiting(store.memberships) { it[1L]?.contains(10L) == true }
        assertEquals(listOf(10L), memberships[1L])
    }

    @Test
    fun aNewGroupReachesTheFlow() = runBlocking {
        store.saveGroup(group(10, "Light + Building 2026"))
        val groups = awaiting(store.groups) { it.isNotEmpty() }
        assertEquals("Light + Building 2026", groups.single().name)
    }

    /**
     * Deleting a group empties the group and keeps the people in it.
     *
     * Asserted through both flows, because "delete does nothing" and "delete
     * takes the contacts with it" look identical from the screen — the group is
     * gone either way — and only one of them is a data-loss bug.
     */
    @Test
    fun deletingAGroupReachesTheFlowAndKeepsTheContacts() = runBlocking {
        store.saveGroup(group(10, "Light + Building 2026"))
        store.save(contact(1, "Jane Okafor"), intoGroup = 10)
        awaiting(store.memberships) { it[1L]?.contains(10L) == true }

        store.deleteGroup(10)

        val groups = awaiting(store.groups) { it.isEmpty() }
        assertTrue("the group survived being deleted", groups.isEmpty())

        val contacts = awaiting(store.contacts) { it.isNotEmpty() }
        assertEquals(
            "the contact was deleted along with its group",
            listOf("Jane Okafor"),
            contacts.map { it.name },
        )

        val memberships = awaiting(store.memberships) { it[1L].isNullOrEmpty() }
        assertTrue("a membership outlived its group", memberships[1L].isNullOrEmpty())
    }

    @Test
    fun removingFromAGroupReachesTheFlow() = runBlocking {
        store.saveGroup(group(10, "Suppliers"))
        store.save(contact(1, "Jane Okafor"), intoGroup = 10)
        awaiting(store.memberships) { it[1L]?.contains(10L) == true }

        store.removeFromGroup(contactId = 1, groupId = 10)

        val memberships = awaiting(store.memberships) { it[1L].isNullOrEmpty() }
        assertTrue(memberships[1L].isNullOrEmpty())
    }

    /**
     * Editing a filed contact keeps it filed.
     *
     * The same rule the schema test covers, asserted here through the store,
     * because this is the call the edit screen actually makes.
     */
    @Test
    fun editingAContactKeepsItInItsGroup() = runBlocking {
        store.saveGroup(group(10, "Suppliers"))
        store.save(contact(1, "Jane Okafor"), intoGroup = 10)
        awaiting(store.memberships) { it[1L]?.contains(10L) == true }

        store.save(contact(1, "Jane Okafor").copy(company = "Meridian Systems Ltd"))

        val contacts = awaiting(store.contacts) { list ->
            list.any { it.company == "Meridian Systems Ltd" }
        }
        assertEquals(1, contacts.size)

        val memberships = awaiting(store.memberships) { it[1L]?.contains(10L) == true }
        assertEquals(
            "editing a contact unfiled it",
            listOf(10L),
            memberships[1L],
        )
    }

    /** Several cards from one photograph, filed together. */
    @Test
    fun everyCardFromOnePhotographIsFiled() = runBlocking {
        store.saveGroup(group(10, "Light + Building 2026"))
        listOf(contact(1, "Jane"), contact(2, "Sam"), contact(3, "Priya"))
            .forEach { store.save(it, intoGroup = 10) }

        val memberships = awaiting(store.memberships) { it.keys.containsAll(listOf(1L, 2L, 3L)) }
        assertTrue(
            "not everyone from the photograph was filed",
            listOf(1L, 2L, 3L).all { memberships[it]?.contains(10L) == true },
        )
    }
}
