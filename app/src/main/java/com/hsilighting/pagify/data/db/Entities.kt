package com.hsilighting.pagify.data.db

import androidx.room.Entity
import androidx.room.ForeignKey
import androidx.room.Index
import androidx.room.PrimaryKey

/**
 * A contact read off a business card.
 *
 * Field names are kept identical to the iOS schema on purpose. The two databases
 * are never synced, but identical shapes mean the same fixtures, the same export
 * output, and a transfer path later that costs nothing now.
 */
@Entity(tableName = "contacts")
data class ContactRow(
    @PrimaryKey val id: Long,
    val name: String,
    val title: String,
    val company: String,
    val address: String,
    val notes: String,
    /**
     * Everything the recogniser produced, never discarded.
     *
     * The largest column by far — half a kilobyte or more per card — and the
     * reason contacts outgrew a whole-file JSON store. It is also what makes
     * search worth having: a phone number the parser failed to classify is still
     * in here.
     */
    val rawText: String,
    val phonesJson: String,
    val emailsJson: String,
    val urlsJson: String,
    val cardImagePath: String?,
    val capturedAt: Long,
    val exportedAt: Long?,
    val exportCount: Int,
    /**
     * Where this one has got to, as a stage name rather than a number.
     *
     * The name is stored, not its position, because positions are what break
     * when a stage is inserted in the middle: every row silently means something
     * else and nothing reports an error. A name that no longer exists reads back
     * as [DealStage.New] and is visible; a number that no longer means what it
     * did is not.
     */
    val stage: String = DealStage.New.stored,
    /**
     * Whether this is somebody met face to face, rather than a card handed on by
     * a colleague or taken from a stand.
     *
     * Its own flag rather than a stage, because it is orthogonal: a contact can
     * be quoted without ever being met, and met without going anywhere.
     */
    val met: Boolean = false,
    /** When to be reminded about them, if ever. */
    val reminderAt: Long? = null,
    /** Set when the reminder has been dealt with, so it stops being due. */
    val reminderDoneAt: Long? = null,
)

/**
 * How far a contact has got, from a card in a pocket to a decision.
 *
 * Fixed rather than user-defined. Editable stages need a migration every time
 * one is renamed, and a rename cannot be told from a delete-plus-add unless
 * identity is tracked separately — a schema of its own, for a thing nobody has
 * asked for twice.
 */
enum class DealStage(val stored: String, val label: String) {
    New("new", "New"),
    Contacted("contacted", "Contacted"),
    Meeting("meeting", "Meeting"),
    Quoted("quoted", "Quoted"),
    Won("won", "Won"),
    Lost("lost", "Lost"),
    ;

    /** Finished with, either way. */
    val isClosed: Boolean get() = this == Won || this == Lost

    companion object {
        /**
         * The stage that string names, or [New].
         *
         * An unknown value falls back rather than throwing. A database written
         * by a newer build has to open in an older one, and losing a stage is
         * recoverable where refusing to open the contact list is not.
         */
        fun of(stored: String?): DealStage = entries.firstOrNull { it.stored == stored } ?: New
    }
}

/**
 * A container the user named: an event, a client, a category.
 *
 * **One entity with an optional date**, rather than separate group, event and
 * date types. A group called "Light + Building 2026" with an event date *is* an
 * event; one called "Hot leads" with no date is a category. The user names the
 * thing and the app imposes no taxonomy on it.
 */
@Entity(tableName = "contact_groups")
data class GroupRow(
    @PrimaryKey val id: Long,
    val name: String,
    /** What lets a group be an event. Null for a plain category. */
    val eventDate: Long?,
    val notes: String,
    /** ARGB, or null. For picking a group out of a list at a glance. */
    val colour: Long?,
    val createdAt: Long,
    /** Set when the whole group is exported together. */
    val lastExportedAt: Long?,
)

/**
 * Which contacts are in which groups.
 *
 * **A join table from day one**, even though the import flow offers one group at
 * a time. Somebody met at an expo may also belong to "Suppliers", and the day
 * that matters a `groupId` column on the contact is a schema migration plus a
 * data backfill — where the UI change on top of a join table is trivial.
 *
 * ## The cascade rule, which is a data-loss bug if it is got wrong
 *
 * Both foreign keys cascade, and what they cascade is **the membership row and
 * nothing else**. Deleting a group removes its memberships; the contacts remain,
 * keep any other memberships they had, and become Ungrouped if that was their
 * last one. A cascade that reached the contact would be the natural thing to
 * write and would silently delete somebody's cards along with the folder they
 * were filed in. There is a test.
 */
@Entity(
    tableName = "group_membership",
    primaryKeys = ["contactId", "groupId"],
    foreignKeys = [
        ForeignKey(
            entity = ContactRow::class,
            parentColumns = ["id"],
            childColumns = ["contactId"],
            onDelete = ForeignKey.CASCADE,
        ),
        ForeignKey(
            entity = GroupRow::class,
            parentColumns = ["id"],
            childColumns = ["groupId"],
            onDelete = ForeignKey.CASCADE,
        ),
    ],
    indices = [Index("groupId"), Index("contactId")],
)
data class MembershipRow(
    val contactId: Long,
    val groupId: Long,
    val addedAt: Long,
)
