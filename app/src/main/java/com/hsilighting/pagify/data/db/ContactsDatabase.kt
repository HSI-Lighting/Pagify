package com.hsilighting.pagify.data.db

import android.content.Context
import androidx.room.Database
import androidx.room.Room
import androidx.room.RoomDatabase
import androidx.room.migration.Migration
import androidx.sqlite.db.SupportSQLiteDatabase

/**
 * Where contacts and their groups live.
 *
 * A database rather than the JSON file the recents and settings use, and the
 * reason is the **write pattern** rather than the size. Forty cards are saved in
 * a row at a trade show; with a whole-file store that is forty complete rewrites,
 * each one a window in which a kill or a flat battery loses the lot. Add half a
 * kilobyte of raw recogniser text per card, and many-to-many membership
 * maintained by hand over parsed JSON, and the simple option stops being simple.
 *
 * This is a **new dependency** — Room and KSP — and worth naming as one rather
 * than slipping in. KSP also needed `android.disallowKotlinSourceSets=false`,
 * because AGP 9 owns Kotlin compilation now and rejects KSP's generated source
 * set by default.
 */
@Database(
    entities = [ContactRow::class, GroupRow::class, MembershipRow::class],
    version = 3,
    exportSchema = false,
)
abstract class ContactsDatabase : RoomDatabase() {

    abstract fun contacts(): ContactsDao

    companion object {
        @Volatile private var instance: ContactsDatabase? = null

        /**
         * Version 2 adds where a contact has got to, and when to be reminded.
         *
         * **Written out rather than left to a destructive fallback.**
         * `fallbackToDestructiveMigration` would make this unnecessary and
         * delete every saved contact on the first launch after the update —
         * which is exactly the data this feature exists to build on. Four
         * `ALTER TABLE ADD COLUMN` statements are the cheapest migration there
         * is and they rewrite nothing.
         *
         * The defaults are stated in SQL as well as in Kotlin. A Kotlin default
         * only applies to rows this app constructs; without the SQL one, every
         * card already saved would have NULL in a column declared non-null.
         */
        val MIGRATION_1_2 = object : Migration(1, 2) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE contacts ADD COLUMN stage TEXT NOT NULL DEFAULT 'new'")
                db.execSQL("ALTER TABLE contacts ADD COLUMN met INTEGER NOT NULL DEFAULT 0")
                db.execSQL("ALTER TABLE contacts ADD COLUMN reminderAt INTEGER")
                db.execSQL("ALTER TABLE contacts ADD COLUMN reminderDoneAt INTEGER")
            }
        }

        /**
         * Version 3 splits one reminder into two: a meeting and a follow-up.
         *
         * The existing `reminderAt` column is kept and becomes the follow-up,
         * so every reminder already set survives as the kind it most likely
         * was. Only the meeting columns are new. SQLite could not rename a
         * column until 3.25 and API 24 ships 3.9, so the alternative was
         * rebuilding the table and copying every row to change a name — see
         * the `@ColumnInfo` on `ContactRow.followUpAt`.
         */
        val MIGRATION_2_3 = object : Migration(2, 3) {
            override fun migrate(db: SupportSQLiteDatabase) {
                db.execSQL("ALTER TABLE contacts ADD COLUMN meetingAt INTEGER")
                db.execSQL("ALTER TABLE contacts ADD COLUMN meetingDoneAt INTEGER")
            }
        }

        fun get(context: Context): ContactsDatabase = instance ?: synchronized(this) {
            instance ?: build(context.applicationContext).also { instance = it }
        }

        private fun build(context: Context) =
            Room.databaseBuilder(context, ContactsDatabase::class.java, "contacts.db")
                .addMigrations(MIGRATION_1_2, MIGRATION_2_3)
                // Write-ahead logging, for the burst this is built for: forty
                // cards saved in a row at an event, while the list on screen is
                // reading the same tables.
                //
                // Foreign keys need no pragma here — Room turns them on itself,
                // which every cascade in this schema relies on.
                .setJournalMode(JournalMode.WRITE_AHEAD_LOGGING)
                .build()
    }
}
