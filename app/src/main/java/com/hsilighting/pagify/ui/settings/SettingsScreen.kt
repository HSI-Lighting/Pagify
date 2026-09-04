package com.hsilighting.pagify.ui.settings

import android.content.pm.PackageManager
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.hsilighting.pagify.core.AppSettings
import com.hsilighting.pagify.core.CardTextSize
import com.hsilighting.pagify.core.ThemeChoice

/**
 * The handful of things that are actually settings.
 *
 * Deliberately short. Everything about *this document* — the tools, the rotation,
 * the page organiser — belongs in the reader where it can be seen taking effect;
 * what is left is what outlives a document.
 */
@Composable
fun SettingsScreen(
    settings: AppSettings,
    onThemeChange: (ThemeChoice) -> Unit,
    onCardTextScale: (Float) -> Unit,
    onShowViewfinder: (Boolean) -> Unit,
    showThumbnails: Boolean,
    onShowThumbnails: (Boolean) -> Unit,
    isRecording: Boolean,
    onToggleRecording: () -> Unit,
    libraryCount: Int,
    onClearLibrary: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var confirmingClear by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val version = remember {
        runCatching {
            context.packageManager.getPackageInfo(context.packageName, 0).versionName
        }.getOrNull().orEmpty()
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 20.dp),
    ) {
        Text(
            "Settings",
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(top = 24.dp, bottom = 16.dp),
        )

        SectionLabel("Appearance")
        SettingCard {
            Column(Modifier.padding(16.dp)) {
                Text(
                    "Theme",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(2.dp))
                Text(
                    "Colours come from your wallpaper on Android 12 and later; " +
                        "this decides whether they are light or dark.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                    ThemeChoice.entries.forEachIndexed { index, choice ->
                        SegmentedButton(
                            selected = settings.theme == choice,
                            onClick = { onThemeChange(choice) },
                            shape = SegmentedButtonDefaults.itemShape(
                                index = index,
                                count = ThemeChoice.entries.size,
                            ),
                            label = { Text(choice.label) },
                        )
                    }
                }
            }
        }

        SettingCard {
            Column(Modifier.padding(16.dp)) {
                Text(
                    "Card review text",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(2.dp))
                Text(
                    "How large the details are on the panel shown after " +
                        "photographing a card. It is read at arm's length, often " +
                        "in poor light and while holding the card.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                    CardTextSize.entries.forEachIndexed { index, size ->
                        SegmentedButton(
                            // Compared with a tolerance rather than by equality:
                            // the value is stored as a float and read back from
                            // JSON, and a stored 1.0 that returns as 0.99999
                            // would leave every option looking unselected.
                            selected = kotlin.math.abs(settings.cardTextScale - size.scale) < 0.01f,
                            onClick = { onCardTextScale(size.scale) },
                            shape = SegmentedButtonDefaults.itemShape(
                                index = index,
                                count = CardTextSize.entries.size,
                            ),
                            label = { Text(size.label) },
                        )
                    }
                }
            }
        }

        // **What the reader cannot do, said plainly.** The recogniser reads
        // Latin script only, and the tempting way to phrase that — "other
        // scripts are not read" — is measured to be wrong. On a bilingual card
        // the other script comes back as Latin-looking nonsense, and because it
        // is usually the largest text at the top of the card it is taken for
        // the person's name. The field is not blank, it is confidently wrong,
        // which is the one outcome somebody skims past. So the notice tells
        // people what to check rather than what is missing.
        SettingCard {
            Column(Modifier.padding(16.dp)) {
                Text(
                    "Scanning reads Latin script",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(2.dp))
                Text(
                    "Phone numbers, emails and websites come through on any " +
                        "card. On a card printed in Arabic as well as English, " +
                        "the Arabic is not read and can be mistaken for the " +
                        "name — so check the name and company before saving.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        SectionLabel("Reading")
        SettingCard {
            ToggleRow(
                title = "Show page thumbnails",
                detail = "The strip beside the page. It hides itself on a narrow screen.",
                checked = showThumbnails,
                onCheckedChange = onShowThumbnails,
            )
        }

        SettingCard {
            ToggleRow(
                title = "Show the viewfinder",
                detail = "The small map of the page that appears while zoomed in, " +
                    "for jumping about without panning. It can also be folded away " +
                    "to a handle from the map itself.",
                checked = settings.showViewfinder,
                onCheckedChange = onShowViewfinder,
            )
        }

        SectionLabel("Library")
        SettingCard {
            ActionRow(
                title = "Clear the library",
                detail = if (libraryCount == 0) {
                    "Nothing to clear."
                } else {
                    "Forget $libraryCount document${if (libraryCount == 1) "" else "s"}. " +
                        "The files themselves are untouched."
                },
                enabled = libraryCount > 0,
                onClick = { confirmingClear = true },
            )
        }

        SectionLabel("Diagnostics")
        SettingCard {
            ToggleRow(
                title = "Record a render timeline",
                detail = "Writes what the reader drew and how long each render took, " +
                    "for chasing a slow or blank page.",
                checked = isRecording,
                onCheckedChange = { onToggleRecording() },
            )
        }

        SectionLabel("About")
        SettingCard {
            Column(Modifier.padding(16.dp)) {
                Text("Pagify", style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
                Spacer(Modifier.height(4.dp))
                Text(
                    if (version.isEmpty()) "PDF reader and markup" else "Version $version",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        Spacer(Modifier.height(32.dp))
    }

    if (confirmingClear) {
        AlertDialog(
            onDismissRequest = { confirmingClear = false },
            title = { Text("Clear the library?") },
            text = {
                Text(
                    "This forgets which documents you have opened. " +
                        "The documents themselves are not touched.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        onClearLibrary()
                        confirmingClear = false
                    },
                ) { Text("Clear") }
            },
            dismissButton = {
                TextButton(onClick = { confirmingClear = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun SectionLabel(text: String) {
    Text(
        text.uppercase(),
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 4.dp, top = 20.dp, bottom = 8.dp),
    )
}

@Composable
private fun SettingCard(content: @Composable () -> Unit) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 1.dp,
        modifier = Modifier.fillMaxWidth(),
    ) { content() }
}

@Composable
private fun ToggleRow(
    title: String,
    detail: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { onCheckedChange(!checked) }
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.height(2.dp))
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Switch(checked = checked, onCheckedChange = onCheckedChange)
    }
}

@Composable
private fun ActionRow(
    title: String,
    detail: String,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, onClick = onClick)
            .padding(16.dp),
    ) {
        Text(
            title,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
            color = if (enabled) {
                MaterialTheme.colorScheme.onSurface
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            },
        )
        Spacer(Modifier.height(2.dp))
        Text(
            detail,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
