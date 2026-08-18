package com.hsilighting.pagify.ui.components

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.ui.Modifier

/**
 * `combinedClickable` without spreading its opt-in across every call site.
 *
 * Isolated here so the experimental annotation sits in one place; if the API
 * stabilises or changes shape, only this file moves.
 */
@OptIn(ExperimentalFoundationApi::class)
fun Modifier.combinedClickableCompat(
    onClick: () -> Unit,
    onLongClick: () -> Unit,
): Modifier = combinedClickable(onClick = onClick, onLongClick = onLongClick)
