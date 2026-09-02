package com.hsilighting.pagify

import androidx.room.Room
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.data.db.ContactRow
import com.hsilighting.pagify.data.db.ContactsDatabase
import com.hsilighting.pagify.data.db.GroupRow
import com.hsilighting.pagify.data.db.MembershipRow
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The contacts schema, and the one rule in it that is a data-loss bug if wrong.
 *
 * An in-memory database, so each test starts empty and nothing touches the
 * device's real contacts.
 */
@RunWith(AndroidJUnit4::class)
class ContactsDatabaseTest {

    private lateinit var database: ContactsDatabase

    @Before
    fun open() {
        database = Room.inMemoryDatabaseBuilder(
            InstrumentationRegistry.getInstrumentation().targetContext,
            ContactsDatabase::class.java,
        ).build()
    }

    @After
    fun close() = database.close()

    private fun contact(id: Long, name: String) = ContactRow(
        id = id,
        name = name,
        title = "",
        company = "",
        address = "",
        notes = "",
        rawText = "",
        phonesJson = "[]",
        emailsJson = "[]",
        urlsJson = "[]",
        cardImagePath = null,
        capturedAt = id,
        exportedAt = null,
        exportCount = 0,
    )

    private fun group(id: Long, name: String) = GroupRow(
        id = id,
        name = name,
        eventDate = null,
        notes = "",
        colour = null,
        createdAt = id,
        lastExportedAt = null,
    )

    /**
     * **Deleting a group must not delete its contacts.**
     *
     * A container that takes its contents with it is the natural thing to write —
     * one cascade too many — and it silently destroys the cards somebody filed.
     * The contact survives, keeps every other group it was in, and becomes
     * ungrouped only if this was its last.
     */
    @Test
    fun deletingAGroupKeepsItsContacts() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))
        dao.saveGroup(group(10, "Light + Building 2026"))
        dao.saveGroup(group(11, "Suppliers"))
        dao.addToGroup(MembershipRow(1, 10, 0))
        dao.addToGroup(MembershipRow(1, 11, 0))

        dao.deleteGroup(10)

        assertEquals(
            "the contact was deleted along with its group",
            1,
            dao.contactsById(listOf(1)).size,
        )
        assertEquals(
            "the other group lost its member too",
            1,
            dao.contactsInGroup(11).size,
        )
        assertTrue(
            "the deleted group still reports members",
            dao.contactsInGroup(10).isEmpty(),
        )
        assertTrue(
            "a contact still in another group should not be ungrouped",
            dao.ungrouped().isEmpty(),
        )
    }

    /** And when it *was* the last group, the contact becomes ungrouped. */
    @Test
    fun aContactWithNoGroupsLeftIsUngrouped() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))
        dao.saveGroup(group(10, "Light + Building 2026"))
        dao.addToGroup(MembershipRow(1, 10, 0))

        dao.deleteGroup(10)

        assertEquals(listOf("Jane Okafor"), dao.ungrouped().map { it.name })
    }

    /** Filing is an aid, not a toll gate: a contact in no group is ordinary. */
    @Test
    fun aContactNeedNoGroupAtAll() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))
        assertEquals(1, dao.ungrouped().size)
    }

    /** Many-to-many from day one, whatever the UI offers. */
    @Test
    fun aContactCanBeInSeveralGroups() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))
        dao.saveGroup(group(10, "Expo"))
        dao.saveGroup(group(11, "Suppliers"))
        dao.addToGroup(MembershipRow(1, 10, 0))
        dao.addToGroup(MembershipRow(1, 11, 0))

        assertEquals(1, dao.contactsInGroup(10).size)
        assertEquals(1, dao.contactsInGroup(11).size)
        assertTrue(dao.ungrouped().isEmpty())
    }

    /**
     * A group export stamps every member with the same instant.
     *
     * They were sent together, so nothing may disagree about when — the contacts,
     * their vCards' `REV`, and the group's own `lastExportedAt` all carry it.
     */
    @Test
    fun exportingAGroupStampsEveryoneTheSame() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))
        dao.save(contact(2, "Sam Reyes"))
        dao.saveGroup(group(10, "Expo"))
        dao.addToGroup(MembershipRow(1, 10, 0))
        dao.addToGroup(MembershipRow(2, 10, 0))

        val at = 1_772_000_000_000L
        dao.markExported(listOf(1, 2), at)
        dao.markGroupExported(10, at)

        val exported = dao.contactsById(listOf(1, 2))
        assertTrue(
            "not everyone carries the same export time",
            exported.all { it.exportedAt == at },
        )
        assertTrue("the export was not counted", exported.all { it.exportCount == 1 })
    }

    /** Exporting twice counts twice, and keeps the later date. */
    @Test
    fun exportingAgainCountsAgain() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))

        dao.markExported(listOf(1), 1_000L)
        dao.markExported(listOf(1), 2_000L)

        val row = dao.contactsById(listOf(1)).single()
        assertEquals(2, row.exportCount)
        assertEquals(2_000L, row.exportedAt)
    }

    /**
     * Merging groups, for the duplicate somebody makes at an event.
     *
     * The contact already in both is the case that breaks a naive move: the
     * join's primary key rejects the duplicate, and without `OR REPLACE` the
     * merge fails for exactly the people who were filed most carefully.
     */
    @Test
    fun mergingMovesEveryoneIncludingThoseInBoth() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))
        dao.save(contact(2, "Sam Reyes"))
        dao.saveGroup(group(10, "Expo"))
        dao.saveGroup(group(11, "Expo 2026"))
        dao.addToGroup(MembershipRow(1, 10, 0))
        dao.addToGroup(MembershipRow(2, 10, 0))
        // Already in both, by hand, before the merge.
        dao.addToGroup(MembershipRow(1, 11, 0))

        dao.merge(from = 10, into = 11)

        assertEquals(2, dao.contactsInGroup(11).size)
        assertTrue("the emptied group survived", dao.contactsInGroup(10).isEmpty())
        assertTrue("somebody was left unfiled", dao.ungrouped().isEmpty())
    }

    /** Removing one membership leaves the others alone. */
    @Test
    fun removingFromOneGroupKeepsTheOther() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))
        dao.saveGroup(group(10, "Expo"))
        dao.saveGroup(group(11, "Suppliers"))
        dao.addToGroup(MembershipRow(1, 10, 0))
        dao.addToGroup(MembershipRow(1, 11, 0))

        dao.removeFromGroup(1, 10)

        assertTrue(dao.contactsInGroup(10).isEmpty())
        assertEquals(1, dao.contactsInGroup(11).size)
    }

    /** Deleting a contact takes its memberships with it, and only those. */
    @Test
    fun deletingAContactLeavesTheGroupStanding() = runBlocking {
        val dao = database.contacts()
        dao.save(contact(1, "Jane Okafor"))
        dao.save(contact(2, "Sam Reyes"))
        dao.saveGroup(group(10, "Expo"))
        dao.addToGroup(MembershipRow(1, 10, 0))
        dao.addToGroup(MembershipRow(2, 10, 0))

        dao.deleteContact(1)

        assertEquals(listOf("Sam Reyes"), dao.contactsInGroup(10).map { it.name })
        assertEquals(1, dao.countIn(10))
    }
}
