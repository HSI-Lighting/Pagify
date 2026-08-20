package com.hsilighting.pagify.ui.components

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector

/**
 * One thing the top bar can do.
 *
 * A list rather than a wall of `IconButton`s so the same actions can be drawn two
 * ways — as icons where there is room, as a menu where there is not — without
 * writing each one twice and letting the two drift.
 */
data class ReaderAction(
    val icon: ImageVector,
    val label: String,
    val tint: Color? = null,
    val onClick: () -> Unit,
)

/**
 * The actions that only appear if the bar is wide enough to hold them.
 *
 * A phone in portrait is around 360dp across. Nine icon buttons at 48dp each need
 * 432, so on a phone the last of them were simply off the edge of the screen —
 * the document could not be opened, which is the first thing anyone would try.
 * The bar keeps undo, redo and save wherever it is; everything else moves into an
 * overflow when there is no room.
 *
 * Whether there is room is asked of the layout rather than assumed from a device
 * class: a tablet in portrait, a phone in landscape and a freeform window are all
 * the same question, and the width answers it for all three.
 */
@Composable
fun ReaderActionBar(hasRoom: Boolean, actions: List<ReaderAction>) {
    if (hasRoom) {
        actions.forEach { action ->
            IconButton(onClick = action.onClick) {
                Icon(
                    imageVector = action.icon,
                    contentDescription = action.label,
                    tint = action.tint ?: androidx.compose.material3.LocalContentColor.current,
                )
            }
        }
        return
    }

    var open by remember { mutableStateOf(false) }
    IconButton(onClick = { open = true }) {
        Icon(Icons.Filled.MoreVert, contentDescription = "More")
    }
    DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
        actions.forEach { action ->
            DropdownMenuItem(
                text = { Text(action.label) },
                leadingIcon = {
                    Icon(
                        imageVector = action.icon,
                        contentDescription = null,
                        tint = action.tint ?: androidx.compose.material3.LocalContentColor.current,
                    )
                },
                onClick = {
                    open = false
                    action.onClick()
                },
            )
        }
    }
}
