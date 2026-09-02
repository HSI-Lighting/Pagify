package com.hsilighting.pagify

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.longClick
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.ContactGroup
import com.hsilighting.pagify.ui.contacts.ContactsScreen
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

/**
 * The screen's own wiring: does tapping a thing call the thing it says it does?
 *
 * The layer nothing covered, and the layer every "it doesn't work" report has
 * turned out to be about. The engine, the schema, the DAO and the store all have
 * tests and all pass; a button wired to nothing passes all of them too.
 *
 * These assert on the **callbacks**, not on what appears afterwards. The screen's
 * job is to report what the user did; whether the result comes back is the
 * store's job, and that is tested separately. A test that checked both at once
 * would not say which half was broken — which is exactly the question that has
 * been costing time.
 */
class ContactsScreenTest {

    @get:Rule val compose = createComposeRule()

    private val jane = Contact(id = 1, name = "Jane Okafor", company = "Meridian")
    private val expo = ContactGroup(id = 10, name = "Light + Building")

    /** Records what the screen asked for, so a test can say what was called. */
    private class Calls {
        var created: String? = null
        var filed: Pair<Long, Long>? = null
        var scannedInto: Long? = null
        var createdForScan: String? = null
        var deleted: ContactGroup? = null
        var scanTarget: Long? = null
        var deletedContacts: List<Contact>? = null
        var deletedGroups: List<ContactGroup>? = null
    }

    private fun show(
        contacts: List<Contact> = listOf(jane),
        groups: List<ContactGroup> = listOf(expo),
        memberships: Map<Long, List<Long>> = emptyMap(),
        pendingFilingLabel: String? = null,
        calls: Calls = Calls(),
    ): Calls {
        compose.setContent {
            ContactsScreen(
                contacts = contacts,
                groups = groups,
                memberships = memberships,
                suggestedGroup = null,
                onCreateGroup = { calls.created = it },
                pendingFilingLabel = pendingFilingLabel,
                onFileScanned = { calls.scannedInto = it },
                onCreateGroupForScan = { calls.createdForScan = it },
                onSkipFiling = {},
                onRenameGroup = { _, _ -> },
                onDeleteGroup = { calls.deleted = it },
                onExportGroup = {},
                onRemoveFromGroup = { _, _ -> },
                onAddToGroupPicked = { contact, group -> calls.filed = contact.id to group },
                onCreateGroupWith = { _, _ -> },
                onScanFromGallery = { calls.scanTarget = it },
                onScanFromCamera = { calls.scanTarget = it },
                onExport = {},
                onDelete = {},
                onSaveEdit = {},
                review = null,
                onKeepReviewed = {},
                onSkipReviewed = {},
                onDeleteContacts = { calls.deletedContacts = it },
                onDeleteGroups = { calls.deletedGroups = it },
                onExportSelected = {},
            )
        }
        return calls
    }

    /**
     * Making a group, from the chip.
     *
     * The chip is drawn unconditionally now. It used to appear only once a
     * contact or a group existed, so a new user could not reach the one control
     * that creates a group — the feature was there and unreachable, which is
     * indistinguishable from absent.
     */
    @Test
    fun theGroupChipCreatesAGroup() {
        val calls = show(contacts = emptyList(), groups = emptyList())

        // The chip creates a group outright now; it used to open a picker
        // that asked which group future cards should go into, which was the
        // same question the prompt after a scan already asks.
        compose.onNodeWithText("Group").performClick()
        compose.onNodeWithText("Name").performTextInput("Expo 2026")
        compose.onNodeWithText("Create").performClick()

        assertEquals("Expo 2026", calls.created)
    }

    /**
     * **Making a group shows the groups.**
     *
     * Reported as "adding the group is stale — I have to close the app and open
     * it again". It was never stale: the view stays on All, where a group cannot
     * be seen, so a new one appeared to do nothing. Restarting re-runs the
     * default and lands on Groups, where it had been all along.
     *
     * It survived every attempt to reproduce because the *first* group flips the
     * default by itself. Only the second onwards, made while on All, vanishes —
     * and tapping Ungrouped is an easy way to end up on All without meaning to.
     */
    @Test
    fun makingAGroupFromTheAllListShowsTheGroups() {
        show()

        compose.onNodeWithText("All").performClick()
        // On All, so the existing group is not on screen at all.
        compose.onNodeWithText("Light + Building").assertDoesNotExist()

        compose.onNodeWithText("Group").performClick()
        compose.onNodeWithText("Name").performTextInput("Expo 2026")
        compose.onNodeWithText("Create").performClick()

        // Back on the list where a group can actually be seen.
        compose.onNodeWithText("Light + Building").assertIsDisplayed()
    }

    /** And it is there with nothing in the app at all. */
    @Test
    fun theGroupChipIsThereBeforeAnythingElseIs() {
        show(contacts = emptyList(), groups = emptyList())
        compose.onNodeWithText("Group").assertIsDisplayed()
    }

    /**
     * Filing a contact that already exists.
     *
     * This did not exist at all: the sheet listed the groups a contact was
     * already in, and tapping one removed it. A contact scanned before any group
     * existed could never be filed.
     */
    @Test
    fun aContactCanBeAddedToAGroup() {
        val calls = show()

        // Groups is the default view once a group exists, so the contacts are
        // behind "All" — worth knowing, and the reason this test looked broken.
        compose.onNodeWithText("All").performClick()
        compose.onNodeWithText("Jane Okafor").performClick()
        compose.onNodeWithText("+ Add to a group").performClick()
        compose.onNodeWithTag("pick:Light + Building").performClick()

        assertEquals(1L to 10L, calls.filed)
    }

    /** Filing what was just scanned, from the prompt that follows a scan. */
    @Test
    fun theFilingPromptFilesWhatWasScanned() {
        val calls = show(pendingFilingLabel = "Jane Okafor")

        compose.onNodeWithTag("pick:Light + Building").performClick()

        assertEquals(10L, calls.scannedInto)
    }

    /** And can make the group it files into, without leaving the flow. */
    @Test
    fun theFilingPromptCanMakeTheGroupItNeeds() {
        val calls = show(groups = emptyList(), pendingFilingLabel = "Jane Okafor")

        compose.onNodeWithText("New group").performClick()
        compose.onNodeWithText("Name").performTextInput("Expo 2026")
        compose.onNodeWithText("Create").performClick()

        assertEquals("Expo 2026", calls.createdForScan)
    }

    /**
     * Deleting a group from the list.
     *
     * Delete used to live only inside an opened group's header. Reasonable once
     * you know; indistinguishable from broken until you do.
     */
    @Test
    fun aGroupCanBeDeletedFromTheList() {
        val calls = show()

        compose.onNodeWithContentDescription("What to do with Light + Building").performClick()
        compose.onNodeWithText("Delete").performClick()
        compose.onNodeWithText("Delete the group").performClick()

        assertEquals(expo, calls.deleted)
    }

    /**
     * Long press picks; a tap then picks another rather than opening one.
     *
     * A list that opens something mid-selection throws the selection away, which
     * is the most irritating way for this to be wrong.
     */
    @Test
    fun longPressPicksSeveralContactsAndDeletesThem() {
        val sam = Contact(id = 2, name = "Sam Reyes")
        val calls = show(contacts = listOf(jane, sam), groups = emptyList())

        compose.onNodeWithText("Jane Okafor").performTouchInput { longClick() }
        compose.onNodeWithText("Sam Reyes").performClick()
        compose.onNodeWithText("2 selected").assertIsDisplayed()

        compose.onNodeWithContentDescription("Delete what is selected").performClick()
        compose.onNodeWithText("Delete").performClick()

        assertEquals(listOf(1L, 2L), calls.deletedContacts?.map { it.id })
    }

    /** Groups are picked the same way, and say plainly what survives. */
    @Test
    fun longPressPicksGroupsAndDeletesThem() {
        val calls = show()

        compose.onNodeWithText("Light + Building").performTouchInput { longClick() }
        compose.onNodeWithText("1 selected").assertIsDisplayed()
        compose.onNodeWithContentDescription("Delete what is selected").performClick()
        compose.onNodeWithText("Delete").performClick()

        assertEquals(listOf(10L), calls.deletedGroups?.map { it.id })
    }

    /** And backing out of a selection clears it rather than leaving the screen. */
    @Test
    fun stoppingSelectionLeavesEverythingAlone() {
        val calls = show()

        compose.onNodeWithText("Light + Building").performTouchInput { longClick() }
        compose.onNodeWithContentDescription("Stop selecting").performClick()

        compose.onNodeWithText("Contacts").assertIsDisplayed()
        assertEquals(null, calls.deletedGroups)
    }
}
