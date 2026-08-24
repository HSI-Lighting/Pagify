package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.SaveAs
import androidx.compose.material3.Badge
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * Save the document, from where the editing happens.
 *
 * The dot is the point of it. Marks and page changes live in memory until they
 * are written, and nothing on screen said so: a reader who highlighted a page and
 * closed the app lost the highlight and had no way of knowing they were about to.
 * The dot says there is unsaved work; its absence says there is not.
 *
 * Save as sits beside it rather than behind a long press. It was reachable only by
 * holding the save button down, which is not a gesture anybody tries on a button
 * that already does something — and it is the only way out for a document that
 * cannot be written to, which a PDF from a mail attachment always is.
 */
@Composable
fun SaveAction(
    hasUnsavedWork: Boolean,
    isSaving: Boolean,
    onSave: () -> Unit,
    onSaveCopy: () -> Unit,
) {
    Box(
        modifier = Modifier.size(48.dp),
        contentAlignment = Alignment.Center,
    ) {
        if (isSaving) {
            // The save itself can take seconds on a large document, and a button
            // that looks idle invites a second press.
            CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
        } else {
            IconButton(
                onClick = onSave,
                enabled = hasUnsavedWork,
            ) {
                Icon(
                    imageVector = Icons.Filled.Save,
                    contentDescription = if (hasUnsavedWork) {
                        "Save the document — there are unsaved changes"
                    } else {
                        "Save the document — everything is saved"
                    },
                )
            }
            if (hasUnsavedWork) {
                Badge(
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .offset(x = (-8).dp, y = 10.dp)
                        .size(8.dp),
                )
            }
        }
    }

    IconButton(
        onClick = onSaveCopy,
        // Always available, unlike Save: a copy of the document as it stands is a
        // reasonable thing to want whether or not anything has been changed.
        enabled = !isSaving,
    ) {
        Icon(
            imageVector = Icons.Filled.SaveAs,
            contentDescription = "Save as a new file",
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
