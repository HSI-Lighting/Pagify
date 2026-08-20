package com.hsilighting.pagify.ui.library

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.colorResource
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.R
import com.hsilighting.pagify.core.RecentDocument
import com.hsilighting.pagify.core.recentSubtitle
import com.hsilighting.pagify.core.searchRecents
import com.hsilighting.pagify.ui.components.combinedClickableCompat
import com.hsilighting.pagify.ui.components.longPressHint

/**
 * What the app opens on: the documents you have read before.
 *
 * The reader used to be the front door, which meant every launch began with an
 * empty grey screen and a file picker — the app asking a question it already knew
 * the answer to. Almost every session is the document from last time, or the one
 * before it.
 *
 * Only documents that actually opened are here. A file that failed, or that asked
 * for a password and never got one, is not something to offer again as though it
 * worked.
 */
@Composable
fun LibraryScreen(
    documents: List<RecentDocument>,
    onOpen: (RecentDocument) -> Unit,
    onForget: (RecentDocument) -> Unit,
    onPickDocument: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var query by remember { mutableStateOf("") }
    val search = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    val shown = remember(documents, query) { searchRecents(documents, query) }

    Box(modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Header(onSearch = { runCatching { search.requestFocus() } })

            Text(
                "Document Library",
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(start = 20.dp, end = 20.dp, bottom = 12.dp),
            )

            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                singleLine = true,
                shape = RoundedCornerShape(14.dp),
                placeholder = { Text("Search files…") },
                leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
                trailingIcon = {
                    if (query.isNotEmpty()) {
                        IconButton(onClick = { query = "" }) {
                            Icon(Icons.Filled.Close, contentDescription = "Clear the search")
                        }
                    }
                },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { keyboard?.hide() }),
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 20.dp)
                    .focusRequester(search),
            )

            Spacer(Modifier.height(12.dp))

            when {
                documents.isEmpty() -> EmptyLibrary(onPickDocument)

                shown.isEmpty() -> NothingMatched(query)

                else -> LazyColumn(
                    // Room at the bottom for the button that floats over it.
                    contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 96.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    items(shown, key = { it.uri }) { document ->
                        DocumentRow(
                            document = document,
                            onOpen = { onOpen(document) },
                            onForget = { onForget(document) },
                        )
                    }
                }
            }
        }

        FloatingActionButton(
            onClick = onPickDocument,
            modifier = Modifier
                .align(Alignment.BottomEnd)
                .padding(20.dp),
        ) {
            Icon(Icons.Filled.Add, contentDescription = "Open a PDF")
        }
    }
}

/**
 * The app's name and mark.
 *
 * The launcher icon rather than a second drawing of it: the thing someone tapped
 * to get here is the thing that should greet them, and two versions of a logo
 * drift apart the moment one of them is edited.
 */
@Composable
private fun Header(onSearch: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 20.dp, vertical = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // The adaptive icon's two layers, composed by hand. `painterResource`
        // cannot load an adaptive icon — it throws "Only VectorDrawables and
        // rasterized asset types are supported" and takes the app down with it.
        //
        // The foreground is drawn at the full size of the circle rather than
        // scaled up the way a launcher mask does: this artwork is already drawn
        // to the edge of its canvas, so enlarging it clips the P and the corner
        // of the page.
        Box(
            modifier = Modifier
                .size(LOGO_SIZE)
                .clip(CircleShape)
                .background(colorResource(R.color.ic_launcher_background)),
            contentAlignment = Alignment.Center,
        ) {
            Image(
                painter = painterResource(R.drawable.ic_launcher_foreground),
                contentDescription = null,
                modifier = Modifier.size(LOGO_SIZE),
            )
        }
        Text(
            "Pagify",
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.Bold,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.weight(1f),
        )
        IconButton(onClick = onSearch) {
            Icon(Icons.Filled.Search, contentDescription = "Search the library")
        }
    }
}

/**
 * One document: what it is called, and enough about it to recognise it.
 *
 * Long press offers to forget it. A row can outlive the file it points at — moved,
 * deleted, or a grant a reboot took away — and without a way to remove it the
 * library slowly fills with rows that only ever fail.
 */
@Composable
private fun DocumentRow(
    document: RecentDocument,
    onOpen: () -> Unit,
    onForget: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }

    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 1.dp,
        shadowElevation = 1.dp,
        modifier = Modifier
            .fillMaxWidth()
            .combinedClickableCompat(onClick = onOpen, onLongClick = { menuOpen = true }),
    ) {
        Row(
            modifier = Modifier.padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier = Modifier
                    .size(46.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(MaterialTheme.colorScheme.surfaceVariant)
                    // The row answers a long press with a menu, and the tile is
                    // the only part of it shaped like an icon.
                    .longPressHint(
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        inset = 4.dp,
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.Filled.Description,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            Column(
                modifier = Modifier
                    .weight(1f)
                    .padding(start = 14.dp),
            ) {
                Text(
                    document.name,
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                val subtitle = recentSubtitle(document)
                if (subtitle.isNotEmpty()) {
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }

            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                DropdownMenuItem(
                    text = { Text("Remove from library") },
                    onClick = {
                        menuOpen = false
                        onForget()
                    },
                )
            }
        }
    }
}

@Composable
private fun EmptyLibrary(onPickDocument: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(
            "Nothing here yet",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.height(6.dp))
        Text(
            "Documents you open show up here, newest first.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(16.dp))
        TextButton(onClick = onPickDocument) { Text("Open a PDF") }
    }
}

@Composable
private fun NothingMatched(query: String) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(32.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(
            "No document called “$query”",
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
        )
        Spacer(Modifier.height(6.dp))
        Text(
            "The library only holds documents you have opened before.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** The mark in the header, matching the size of a toolbar icon and its label. */
private val LOGO_SIZE = 34.dp

