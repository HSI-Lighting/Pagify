package com.hsilighting.pagify

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.hsilighting.pagify.ui.reader.PdfReaderScreen
import com.hsilighting.pagify.ui.reader.PdfReaderViewModel
import com.hsilighting.pagify.ui.theme.PagifyTheme
import kotlinx.coroutines.flow.MutableStateFlow

class MainActivity : ComponentActivity() {

    /**
     * A PDF handed to us by another app (a mail attachment, a browser download).
     *
     * Held in a flow rather than read once in `onCreate` so that `onNewIntent` —
     * which fires when the activity is already running — reaches the same handler.
     */
    private val incomingDocument = MutableStateFlow<Uri?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        incomingDocument.value = viewableUri(intent)

        setContent {
            PagifyTheme {
                val viewModel: PdfReaderViewModel = viewModel()
                val state by viewModel.state.collectAsStateWithLifecycle()
                val incoming by incomingDocument.collectAsStateWithLifecycle()

                LaunchedEffect(incoming) {
                    incoming?.let { uri ->
                        viewModel.open(uri)
                        // Cleared so a configuration change does not reopen it and
                        // discard the page the user had scrolled to.
                        incomingDocument.value = null
                    }
                }

                val picker = rememberLauncherForActivityResult(
                    ActivityResultContracts.OpenDocument(),
                ) { uri ->
                    if (uri != null) {
                        // Without this the grant expires with the process, so a
                        // document reopened after a low-memory kill would fail.
                        runCatching {
                            contentResolver.takePersistableUriPermission(
                                uri,
                                Intent.FLAG_GRANT_READ_URI_PERMISSION,
                            )
                        }
                        viewModel.open(uri)
                    }
                }

                val openPicker = remember { { picker.launch(arrayOf(PDF_MIME_TYPE)) } }

                PdfReaderScreen(
                    state = state,
                    onPickDocument = openPicker,
                    onPageVisible = viewModel::onPageVisible,
                    onZoomInOn = viewModel::zoomInOn,
                    onZoomTo = viewModel::zoomTo,
                    onViewportWidth = viewModel::onViewportWidthChanged,
                    onRotate = viewModel::rotate,
                    onToggleThumbnails = viewModel::toggleThumbnails,
                    onToggleRecording = {
                        // App-private external storage, so the file can be pulled
                        // with adb without any permission prompt.
                        viewModel.toggleRecording(getExternalFilesDir(null))?.let { message ->
                            Toast.makeText(this, message, Toast.LENGTH_LONG).show()
                        }
                    },
                    onShowMetadata = viewModel::showMetadata,
                    onSubmitPassword = viewModel::submitPassword,
                    pageSizeProvider = viewModel::pageSize,
                    renderer = viewModel::renderPage,
                    thumbnailRenderer = viewModel::renderThumbnail,
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        viewableUri(intent)?.let { incomingDocument.value = it }
    }

    private fun viewableUri(intent: Intent?): Uri? = intent
        ?.takeIf { it.action == Intent.ACTION_VIEW || it.action == Intent.ACTION_SEND }
        ?.data

    companion object {
        const val PDF_MIME_TYPE = "application/pdf"
    }
}
