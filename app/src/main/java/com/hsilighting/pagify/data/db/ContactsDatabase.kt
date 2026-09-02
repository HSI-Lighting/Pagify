package com.hsilighting.pagify.data.db

import android.content.Context
import androidx.room.Database
import androidx.room.Room
import androidx.room.RoomDatabase

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
    version = 1,
    exportSchema = false,
)
abstract class ContactsDatabase : RoomDatabase() {

    abstract fun contacts(): ContactsDao

    companion object {
        @Volatile private var instance: ContactsDatabase? = null

        fun get(context: Context): ContactsDatabase = instance ?: synchronized(this) {
            instance ?: build(context.applicationContext).also { instance = it }
        }

        private fun build(context: Context) =
            Room.databaseBuilder(context, ContactsDatabase::class.java, "contacts.db")
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
