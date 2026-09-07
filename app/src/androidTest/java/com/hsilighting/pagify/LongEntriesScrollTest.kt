package com.hsilighting.pagify

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.Meeting
import com.hsilighting.pagify.core.Phone
import com.hsilighting.pagify.ui.contacts.ContactSheet
import com.hsilighting.pagify.ui.contacts.ProgressSheet
import java.util.Calendar
import org.junit.Rule
import org.junit.Test

/**
 * A long entry has to be reachable to the end of it.
 *
 * **A dialog does not scroll its own body.** Material's `AlertDialog` caps its
 * height against the window and then simply cuts off whatever will not fit —
 * silently, with nothing on screen to suggest there is more. A contact with
 * four numbers and a paragraph of notes lost its last details, and a contact
 * with a few meetings arranged lost the Save button itself, which made the sheet
 * not merely awkward but impossible to finish.
 *
 * `performScrollTo` is the assertion that matters here: it **throws** when no
 * scrollable ancestor exists, so these fail on the old layout rather than
 * passing by accident on a tall test device. That is the whole point — the bug
 * only shows on a screen small enough, and a test that quietly passed on a big
 * one would be worse than none.
 */
class LongEntriesScrollTest {

    @get:Rule
    val rule = createComposeRule()

    private val hour = 3_600_000L
    private val now = System.currentTimeMillis()

    /** A card that gave up everything it had: the realistic worst case. */
    private fun crowdedContact() = Contact(
        id = 1,
        name = "Priya Raman",
        title = "Head of Purchasing and Supplier Relations",
        company = "Northwind Traders International",
        address = "Fourth Floor, Meridian House, 44 Harbour Road, Mumbai 400001",
        notes = "Met at the trade show. Wants the full catalogue and a quotation " +
            "for the spring order. Prefers email. Back from leave in April.",
        phones = listOf(
            Phone(raw = "+91 22 5555 0101", normalised = "+912255550101", kind = "work"),
            Phone(raw = "+91 22 5555 0102", normalised = "+912255550102", kind = "office"),
            Phone(raw = "+91 98200 55501", normalised = "+919820055501", kind = "mobile"),
            Phone(raw = "+91 22 5555 0199", normalised = "+912255550199", kind = "fax"),
        ),
        emails = listOf(
            "priya.raman@northwind.example",
            "purchasing@northwind.example",
            "p.raman@northwind-intl.example",
        ),
        urls = listOf("https://northwind.example", "https://northwind-intl.example/purchasing"),
    )

    /**
     * The bottom of a crowded contact can be reached.
     *
     * The notes are last, so they are what falls off the end.
     */
    @Test
    fun the_end_of_a_long_contact_is_reachable() {
        rule.setContent {
            ContactSheet(
                contact = crowdedContact(),
                groups = emptyList(),
                onEdit = {},
                onProgress = {},
                onExport = {},
                onDelete = {},
                onRemoveFromGroup = {},
                onAddToGroup = {},
                onDismiss = {},
            )
        }

        rule.onNodeWithText("Met at the trade show", substring = true)
            .performScrollTo()
            .assertIsDisplayed()
    }

    /** And so is a detail in the middle of the run of numbers. */
    @Test
    fun a_number_below_the_fold_is_reachable() {
        rule.setContent {
            ContactSheet(
                contact = crowdedContact(),
                groups = emptyList(),
                onEdit = {},
                onProgress = {},
                onExport = {},
                onDelete = {},
                onRemoveFromGroup = {},
                onAddToGroup = {},
                onDismiss = {},
            )
        }

        rule.onNodeWithText("+91 22 5555 0199", substring = true)
            .performScrollTo()
            .assertIsDisplayed()
    }

    /**
     * The progress sheet stays usable as meetings pile up.
     *
     * This one is worse than losing a detail: the follow-up row is below the
     * meetings, so a contact seen often enough pushed it off the screen with no
     * way to bring it back.
     */
    @Test
    fun the_follow_up_row_survives_a_pile_of_meetings() {
        val busy = Contact(
            id = 2,
            name = "Owen Hart",
            company = "Ridgeway",
            meetings = (1..8).map { Meeting(id = it.toLong(), contactId = 2, at = now + it * 24 * hour) },
        )

        rule.setContent {
            ProgressSheet(contact = busy, onSave = {}, onDismiss = {})
        }

        // The heading is upper-cased for style, so match on meaning not on case.
        rule.onNodeWithText("Follow up", ignoreCase = true).performScrollTo().assertIsDisplayed()
        rule.onNodeWithText("Waits quietly in the notification shade.")
            .performScrollTo()
            .assertIsDisplayed()
    }

    /** Every arranged meeting can be reached, including the last. */
    @Test
    fun the_last_of_many_meetings_is_reachable() {
        val at = Calendar.getInstance().apply {
            add(Calendar.DAY_OF_YEAR, 8)
            set(Calendar.HOUR_OF_DAY, 15)
            set(Calendar.MINUTE, 0)
            set(Calendar.SECOND, 0)
            set(Calendar.MILLISECOND, 0)
        }.timeInMillis

        val busy = Contact(
            id = 3,
            name = "Owen Hart",
            meetings = (1..7).map { Meeting(id = it.toLong(), contactId = 3, at = now + it * 24 * hour) } +
                Meeting(id = 8, contactId = 3, at = at),
        )

        rule.setContent {
            ProgressSheet(contact = busy, onSave = {}, onDismiss = {})
        }

        rule.onNodeWithText(com.hsilighting.pagify.ui.contacts.whenItIs(at))
            .performScrollTo()
            .assertIsDisplayed()
    }
}
