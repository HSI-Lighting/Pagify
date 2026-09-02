package com.hsilighting.pagify.ui

import androidx.compose.runtime.LaunchedEffect
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarHost
import com.hsilighting.pagify.ui.contacts.ContactsScreen
import com.hsilighting.pagify.core.Contact
import com.hsilighting.pagify.core.ContactGroup
import androidx.compose.material.icons.filled.ContactPage
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.automirrored.filled.LibraryBooks
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import com.hsilighting.pagify.core.AppSettings
import com.hsilighting.pagify.core.RecentDocument
import com.hsilighting.pagify.core.ThemeChoice
import com.hsilighting.pagify.ui.library.LibraryScreen
import com.hsilighting.pagify.ui.reader.PdfReaderState
import com.hsilighting.pagify.ui.settings.SettingsScreen

/**
 * The two places the app can be: in a document, or not.
 *
 * The tab bar belongs to "not". A reader with a tab bar under it is a reader with
 * a strip of its page missing — and the page is the whole point — so opening a
 * document takes the screen and closing it gives the tabs back. Back is what
 * closes it, which is the gesture people already use for exactly this.
 *
 * The reader arrives as a slot rather than as arguments. It takes some sixty
 * callbacks, and threading those through here would make this file a copy of the
 * wiring in `MainActivity` that has to be edited every time the reader gains a
 * control.
 */
@Composable
fun PagifyApp(
    state: PdfReaderState,
    recents: List<RecentDocument>,
    onOpenRecent: (RecentDocument) -> Unit,
    onForgetRecent: (RecentDocument) -> Unit,
    onPickDocument: () -> Unit,
    onClearLibrary: () -> Unit,
    onShowThumbnails: (Boolean) -> Unit,
    settings: AppSettings,
    onThemeChange: (ThemeChoice) -> Unit,
    onShowViewfinder: (Boolean) -> Unit,
    onToggleRecording: () -> Unit,
    onReturnToLibrary: () -> Unit,
    /** Contacts read off business cards. */
    contacts: List<Contact>,
    contactGroups: List<ContactGroup>,
    /** Contact id to the groups it is in. */
    groupMemberships: Map<Long, List<Long>>,
    /** The group newly scanned cards are filed into, or null for none. */
    importTarget: Long?,
    onSetImportTarget: (Long?) -> Unit,
    onCreateGroup: (String) -> Unit,
    /** What was just scanned and is waiting to be filed, if anything. */
    pendingFilingLabel: String?,
    onFileScanned: (Long) -> Unit,
    onCreateGroupForScan: (String) -> Unit,
    onSkipFiling: () -> Unit,
    onRenameGroup: (ContactGroup, String) -> Unit,
    onDeleteGroup: (ContactGroup) -> Unit,
    onExportGroup: (ContactGroup) -> Unit,
    onRemoveFromGroup: (Contact, Long) -> Unit,
    /** From the gallery, and from the camera. */
    onScanCard: () -> Unit,
    onPhotographCard: () -> Unit,
    onExportContact: (Contact) -> Unit,
    onDeleteContact: (Contact) -> Unit,
    onSaveContact: (Contact) -> Unit,
    /** Cleared once shown, so the same message can be sent twice. */
    onMessageShown: () -> Unit,
    reader: @Composable () -> Unit,
) {
    // A document is open — or opening, or asking for a password, or has failed to
    // open, all of which the reader has something to say about.
    val inDocument = state.phase != PdfReaderState.Phase.Empty

    if (inDocument) {
        // Back closes the document rather than the app. Registered here rather
        // than inside the reader so it cannot fight the capture editor's own
        // handler, which sits in a dialog window of its own.
        BackHandler(onBack = onReturnToLibrary)
        Box(Modifier.fillMaxSize()) { reader() }
        return
    }

    // Survives rotation: turning the phone while reading the settings should not
    // quietly put you back in the library.
    var tab by rememberSaveable { mutableStateOf(HomeTab.Library) }

    // The home screens had no way to say anything at all. Everything that
    // reports an outcome — scanning a card, exporting a contact — put its
    // message into the reader's snackbar, which does not exist outside a
    // document. So a scan that found no QR code was indistinguishable from a
    // scan that never ran: the app simply did nothing, twice over.
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(state.message) {
        val message = state.message ?: return@LaunchedEffect
        snackbar.showSnackbar(message)
        onMessageShown()
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = {
            NavigationBar {
                HomeTab.entries.forEach { entry ->
                    NavigationBarItem(
                        selected = tab == entry,
                        onClick = { tab = entry },
                        icon = { Icon(entry.icon, contentDescription = null) },
                        label = { Text(entry.label) },
                    )
                }
            }
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when (tab) {
                HomeTab.Library -> LibraryScreen(
                    documents = recents,
                    onOpen = onOpenRecent,
                    onForget = onForgetRecent,
                    onPickDocument = onPickDocument,
                )

                HomeTab.Contacts -> ContactsScreen(
                    contacts = contacts,
                    groups = contactGroups,
                    memberships = groupMemberships,
                    importTarget = importTarget,
                    onSetImportTarget = onSetImportTarget,
                    onCreateGroup = onCreateGroup,
                    pendingFilingLabel = pendingFilingLabel,
                    onFileScanned = onFileScanned,
                    onCreateGroupForScan = onCreateGroupForScan,
                    onSkipFiling = onSkipFiling,
                    onRenameGroup = onRenameGroup,
                    onDeleteGroup = onDeleteGroup,
                    onExportGroup = onExportGroup,
                    onRemoveFromGroup = onRemoveFromGroup,
                    onScanFromGallery = onScanCard,
                    onScanFromCamera = onPhotographCard,
                    onExport = onExportContact,
                    onDelete = onDeleteContact,
                    onSaveEdit = onSaveContact,
                )

                HomeTab.Settings -> SettingsScreen(
                    showThumbnails = state.showThumbnails,
                    settings = settings,
                    onThemeChange = onThemeChange,
                    onShowViewfinder = onShowViewfinder,
                    onShowThumbnails = onShowThumbnails,
                    isRecording = state.isRecording,
                    onToggleRecording = onToggleRecording,
                    libraryCount = recents.size,
                    onClearLibrary = onClearLibrary,
                )
            }
        }
    }
}

/**
 * Where the tab bar can take you.
 *
 * Three, and each earns its slot: the library is the app's front door,
 * contacts is a separate body of the user's own data rather than a view of a
 * document, and settings is the only other thing that outlives a document. A
 * bar padded out with places that are really one screen is a bar that teaches
 * people to ignore it.
 */
enum class HomeTab(val label: String, val icon: ImageVector) {
    Library("Library", Icons.AutoMirrored.Filled.LibraryBooks),
    Contacts("Contacts", Icons.Filled.ContactPage),
    Settings("Settings", Icons.Filled.Settings),
}
