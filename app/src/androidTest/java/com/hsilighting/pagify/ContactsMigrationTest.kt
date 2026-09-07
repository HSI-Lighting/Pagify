package com.hsilighting.pagify

import androidx.room.Room
import androidx.sqlite.db.framework.FrameworkSQLiteOpenHelperFactory
import androidx.sqlite.db.SupportSQLiteDatabase
import androidx.sqlite.db.SupportSQLiteOpenHelper
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.data.db.ContactsDatabase
import com.hsilighting.pagify.data.db.DealStage
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test

/**
 * The migration that must not lose anybody's contacts.
 *
 * **This is the one test in the suite guarding data that cannot be regenerated.**
 * A card is photographed once, at an event, and the photograph is usually gone;
 * the row in this database is the only copy. Room offers
 * `fallbackToDestructiveMigration`, which would have made version 2 a one-line
 * change and silently deleted every saved contact on the first launch after the
 * update — and the failure would look like an app that had simply forgotten,
 * with nothing to say why.
 *
 * So the migration is written out, and this builds a real version 1 database
 * with a real row in it, runs the real migration, and reads the row back.
 *
 * The v1 schema below is copied from the shipped one rather than generated. That
 * is deliberate: if the entity is edited and this is not, the test fails at the
 * insert, which is exactly the moment somebody should be asked whether a
 * migration is needed.
 */
class ContactsMigrationTest {

    private val name = "migration-probe.db"

    @Before
    fun clearAny() {
        InstrumentationRegistry.getInstrumentation().targetContext.deleteDatabase(name)
    }

    /** The `contacts` table exactly as version 1 shipped it. */
    private fun createVersionOne(): SupportSQLiteDatabase {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val configuration = SupportSQLiteOpenHelper.Configuration.builder(context)
            .name(name)
            .callback(object : SupportSQLiteOpenHelper.Callback(1) {
                override fun onCreate(db: SupportSQLiteDatabase) {
                    db.execSQL(
                        """
                        CREATE TABLE IF NOT EXISTS `contacts` (
                            `id` INTEGER NOT NULL,
                            `name` TEXT NOT NULL,
                            `title` TEXT NOT NULL,
                            `company` TEXT NOT NULL,
                            `address` TEXT NOT NULL,
                            `notes` TEXT NOT NULL,
                            `rawText` TEXT NOT NULL,
                            `phonesJson` TEXT NOT NULL,
                            `emailsJson` TEXT NOT NULL,
                            `urlsJson` TEXT NOT NULL,
                            `cardImagePath` TEXT,
                            `capturedAt` INTEGER NOT NULL,
                            `exportedAt` INTEGER,
                            `exportCount` INTEGER NOT NULL,
                            PRIMARY KEY(`id`)
                        )
                        """.trimIndent(),
                    )
                    // The other two tables as version 1 shipped them. They are
                    // untouched by this migration and are here because Room
                    // validates the *whole* schema after migrating, not just the
                    // table that changed — leaving them out failed this test
                    // with "Migration didn't properly handle: contact_groups",
                    // which is the validator working rather than the migration
                    // breaking.
                    db.execSQL(
                        """
                        CREATE TABLE IF NOT EXISTS `contact_groups` (
                            `id` INTEGER NOT NULL,
                            `name` TEXT NOT NULL,
                            `eventDate` INTEGER,
                            `notes` TEXT NOT NULL,
                            `colour` INTEGER,
                            `createdAt` INTEGER NOT NULL,
                            `lastExportedAt` INTEGER,
                            PRIMARY KEY(`id`)
                        )
                        """.trimIndent(),
                    )
                    db.execSQL(
                        """
                        CREATE TABLE IF NOT EXISTS `group_membership` (
                            `contactId` INTEGER NOT NULL,
                            `groupId` INTEGER NOT NULL,
                            `addedAt` INTEGER NOT NULL,
                            PRIMARY KEY(`contactId`, `groupId`),
                            FOREIGN KEY(`contactId`) REFERENCES `contacts`(`id`)
                                ON UPDATE NO ACTION ON DELETE CASCADE ,
                            FOREIGN KEY(`groupId`) REFERENCES `contact_groups`(`id`)
                                ON UPDATE NO ACTION ON DELETE CASCADE
                        )
                        """.trimIndent(),
                    )
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS `index_group_membership_groupId` " +
                            "ON `group_membership` (`groupId`)",
                    )
                    db.execSQL(
                        "CREATE INDEX IF NOT EXISTS `index_group_membership_contactId` " +
                            "ON `group_membership` (`contactId`)",
                    )
                }

                override fun onUpgrade(db: SupportSQLiteDatabase, old: Int, new: Int) = Unit
            })
            .build()
        return FrameworkSQLiteOpenHelperFactory().create(configuration).writableDatabase
    }

    /**
     * A contact saved before the update is still there afterwards, unchanged,
     * with the new columns at their defaults.
     */
    @Test
    fun a_contact_saved_before_the_update_survives_it() {
        createVersionOne().use { old ->
            old.execSQL(
                """
                INSERT INTO contacts VALUES (
                    77, 'Priya Raman', 'Head of Purchasing', 'Northwind Traders', '',
                    'a note', 'raw recogniser text', '[]', '["priya@northwind.example"]',
                    '[]', NULL, 1700000000000, NULL, 0
                )
                """.trimIndent(),
            )
            // Room writes its own identity row and refuses to open a database
            // without one. Version 1 is what this database claims to be.
            old.version = 1
        }

        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val database = Room.databaseBuilder(context, ContactsDatabase::class.java, name)
            // **Both migrations, because a version 1 database has to reach 3.**
            // Registering only the first fails with "A migration from 1 to 3
            // was required but not found" -- which is this test earning its
            // place: somebody who added version 3 and forgot the path from 1
            // would have shipped an app that refuses to open for every user
            // who had not updated in between.
            .addMigrations(ContactsDatabase.MIGRATION_1_2, ContactsDatabase.MIGRATION_2_3)
            .build()

        try {
            val contact = runBlocking { database.contacts().contactsById(listOf(77)) }.single()

            // Everything that was there is still there.
            assertEquals("Priya Raman", contact.name)
            assertEquals("Head of Purchasing", contact.title)
            assertEquals("Northwind Traders", contact.company)
            assertEquals("a note", contact.notes)
            assertEquals("raw recogniser text", contact.rawText)
            assertEquals("""["priya@northwind.example"]""", contact.emailsJson)
            assertEquals(1700000000000L, contact.capturedAt)

            // And the new columns hold their defaults rather than nulls. `stage`
            // is declared non-null, so a column added without a SQL default
            // would fail to read at all.
            assertEquals(DealStage.New.stored, contact.stage)
            assertFalse(contact.met)
            assertNull(contact.followUpAt)
            assertNull(contact.followUpDoneAt)
            // Added by version 3, and null for a row that predates it.
            assertNull(contact.meetingAt)
            assertNull(contact.meetingDoneAt)
        } finally {
            database.close()
            context.deleteDatabase(name)
        }
    }
}
