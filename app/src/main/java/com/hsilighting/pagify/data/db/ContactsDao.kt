package com.hsilighting.pagify.data.db

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.Transaction
import kotlinx.coroutines.flow.Flow

@Dao
interface ContactsDao {

    // ------------------------------------------------------------ contacts --

    @Query("SELECT * FROM contacts ORDER BY capturedAt DESC")
    fun contacts(): Flow<List<ContactRow>>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun save(contact: ContactRow)

    @Query("DELETE FROM contacts WHERE id = :id")
    suspend fun deleteContact(id: Long)

    @Query("SELECT * FROM contacts WHERE id IN (:ids)")
    suspend fun contactsById(ids: List<Long>): List<ContactRow>

    /**
     * Record that these contacts were exported, all at the same instant.
     *
     * One statement rather than a read-modify-write per contact: a group export
     * of forty people should be one transaction, and every member must carry the
     * *same* timestamp — they were sent together, and the group's own
     * `lastExportedAt` has to agree with theirs.
     */
    @Query(
        "UPDATE contacts SET exportedAt = :at, exportCount = exportCount + 1 " +
            "WHERE id IN (:ids)",
    )
    suspend fun markExported(ids: List<Long>, at: Long)

    // -------------------------------------------------------------- groups --

    @Query("SELECT * FROM contact_groups ORDER BY createdAt DESC")
    fun groups(): Flow<List<GroupRow>>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun saveGroup(group: GroupRow)

    /**
     * Delete a group.
     *
     * Its membership rows go with it, by the foreign key's cascade. **Its
     * contacts do not** — they keep every other group they were in, and become
     * Ungrouped if this was their last. See the note on [MembershipRow].
     */
    @Query("DELETE FROM contact_groups WHERE id = :id")
    suspend fun deleteGroup(id: Long)

    @Query("UPDATE contact_groups SET lastExportedAt = :at WHERE id = :id")
    suspend fun markGroupExported(id: Long, at: Long)

    // ---------------------------------------------------------- membership --

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun addToGroup(membership: MembershipRow)

    @Query("DELETE FROM group_membership WHERE contactId = :contactId AND groupId = :groupId")
    suspend fun removeFromGroup(contactId: Long, groupId: Long)

    @Query("SELECT * FROM group_membership")
    fun memberships(): Flow<List<MembershipRow>>

    @Query(
        "SELECT c.* FROM contacts c " +
            "JOIN group_membership m ON m.contactId = c.id " +
            "WHERE m.groupId = :groupId ORDER BY c.capturedAt DESC",
    )
    suspend fun contactsInGroup(groupId: Long): List<ContactRow>

    /**
     * Contacts filed nowhere.
     *
     * A real state, not an error: filing is an aid, never a toll gate on saving a
     * card.
     */
    @Query(
        "SELECT * FROM contacts WHERE id NOT IN " +
            "(SELECT contactId FROM group_membership) ORDER BY capturedAt DESC",
    )
    suspend fun ungrouped(): List<ContactRow>

    @Query("SELECT COUNT(*) FROM group_membership WHERE groupId = :groupId")
    suspend fun countIn(groupId: Long): Int

    /**
     * Move every membership from one group into another, then drop the emptied
     * one.
     *
     * For the duplicate group somebody creates by accident at an event, when the
     * name is nearly the same and the cards are split across both.
     *
     * `OR REPLACE` because a contact can already be in both, and the join's
     * primary key would otherwise reject the move for exactly the people who were
     * filed most carefully.
     */
    @Transaction
    suspend fun merge(from: Long, into: Long) {
        moveMemberships(from, into)
        deleteGroup(from)
    }

    @Query(
        "INSERT OR REPLACE INTO group_membership (contactId, groupId, addedAt) " +
            "SELECT contactId, :into, addedAt FROM group_membership WHERE groupId = :from",
    )
    suspend fun moveMemberships(from: Long, into: Long)
}
