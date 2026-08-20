package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Save
import androidx.compose.material3.Badge
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
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
 * A long press offers "Save a copy" rather than a menu on every tap, because
 * saving is the common case by a wide margin. It is also the way out when the
 * document cannot be written to — a PDF opened from a mail attachment arrives
 * read-only, and no amount of trying the first option will change that.
 */
@Composable
fun SaveAction(
    hasUnsavedWork: Boolean,
    isSaving: Boolean,
    onSave: () -> Unit,
    onSaveCopy: () -> Unit,
) {
    var showingMenu by remember { mutableStateOf(false) }

    Box {
        Box(
            modifier = Modifier
                .size(48.dp)
                .combinedClickableCompat(
                    onClick = { if (hasUnsavedWork && !isSaving) onSave() },
                    // Available even with nothing unsaved: "save a copy" of the
                    // document as it stands is a reasonable thing to want.
                    onLongClick = { if (!isSaving) showingMenu = true },
                ),
            contentAlignment = Alignment.Center,
        ) {
            if (isSaving) {
                // The save itself can take seconds on a large document, and a
                // button that looks idle invites a second press.
                CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
            } else {
                Icon(
                    imageVector = Icons.Filled.Save,
                    contentDescription = if (hasUnsavedWork) {
                        "Save the document — there are unsaved changes"
                    } else {
                        "Save a copy"
                    },
                    tint = if (hasUnsavedWork) {
                        MaterialTheme.colorScheme.onSurface
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                )
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

        DropdownMenu(expanded = showingMenu, onDismissRequest = { showingMenu = false }) {
            DropdownMenuItem(
                text = { Text("Save") },
                enabled = hasUnsavedWork,
                onClick = {
                    showingMenu = false
                    onSave()
                },
            )
            DropdownMenuItem(
                text = { Text("Save a copy…") },
                onClick = {
                    showingMenu = false
                    onSaveCopy()
                },
            )
        }
    }
}
