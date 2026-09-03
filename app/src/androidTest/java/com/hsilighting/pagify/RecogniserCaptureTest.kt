package com.hsilighting.pagify

import android.net.Uri
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.text.TextRecognition
import com.google.mlkit.vision.text.latin.TextRecognizerOptions
import com.hsilighting.pagify.core.RecogniserDump
import com.google.android.gms.tasks.Tasks
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Capture what the recogniser returns for a folder of photographs.
 *
 * Not a test — a **tool**, which is why it is ignored by default. It exists
 * because driving the app's own UI to capture one card takes a dozen taps, is
 * defeated by whichever dialog happens to be on screen, and cannot be repeated
 * reliably enough to build a corpus from.
 *
 * This runs ML Kit directly over every image in a folder and writes one capture
 * per photograph, so thirty cards are one command rather than an afternoon.
 *
 * ## Running it
 *
 * ```
 * adb push <cards> /sdcard/Android/data/com.hsilighting.pagify.debug/files/cards/
 * gradlew connectedDebugAndroidTest \
 *   -Pandroid.testInstrumentationRunnerArguments.class=\
 *      com.hsilighting.pagify.RecogniserCaptureTest#capture
 * adb pull /sdcard/Android/data/com.hsilighting.pagify.debug/files/recogniser
 * ```
 *
 * ## Why it skips rather than being ignored
 *
 * It reads whatever photographs are on the device and writes their text to disk —
 * on a real card, somebody's name and telephone number. So it does nothing unless
 * the folder has been staged deliberately, which makes it inert in an ordinary
 * suite run and available by name when it is wanted. `@Ignore` would have been
 * the obvious choice and is the wrong one: it cannot be run by name either.
 */
@RunWith(AndroidJUnit4::class)
class RecogniserCaptureTest {

    @Test
    fun capture() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext

        // Granted here rather than by hand: the Gradle task reinstalls the APK
        // immediately before running, and a reinstall drops every runtime
        // permission — so an `adb shell pm grant` is gone before the first line
        // of this executes, and the run silently skips.
        runCatching {
            instrumentation.uiAutomation.grantRuntimePermission(
                context.packageName,
                "android.permission.READ_MEDIA_IMAGES",
            )
        }
        // Shared storage, which the *shipped* app cannot read: it asks for no
        // storage permission at all and reaches every file through the system
        // pickers. The debug manifest adds the permission for this tool alone —
        // see `app/src/debug/AndroidManifest.xml` — and it must be granted:
        //     adb shell pm grant com.hsilighting.pagify.debug \n        //         android.permission.READ_MEDIA_IMAGES
        val folder = File("/sdcard/Download/cards")
        val images = folder.listFiles { file ->
            file.isFile && file.name.substringAfterLast('.', "").lowercase() in FORMATS
        }?.sortedBy { it.name }.orEmpty()

        assumeTrue("no photographs in ${folder.absolutePath}", images.isNotEmpty())

        val recogniser = TextRecognition.getClient(TextRecognizerOptions.DEFAULT_OPTIONS)
        try {
            images.forEach { image ->
                val input = InputImage.fromFilePath(context, Uri.fromFile(image))
                // Blocking on purpose: this is a tool run to completion, and the
                // alternative is a callback dance that adds nothing here.
                val text = Tasks.await(recogniser.process(input))
                RecogniserDump.write(context, text, image.nameWithoutExtension)

                // Printed as well as written. On Android 11 and later `adb shell`
                // cannot list another app's `Android/data`, so the file is there
                // and unreachable; test output is the one channel that always
                // comes back.
                Log.i("CardCapture", "### ${image.name}")
                text.textBlocks.flatMap { it.lines }
                    .sortedBy { (it.boundingBox?.top ?: 0) + (it.boundingBox?.bottom ?: 0) }
                    .forEach { line ->
                        val b = line.boundingBox
                        Log.i(
                            "CardCapture",
                            "LINE cy=${((b?.top ?: 0) + (b?.bottom ?: 0)) / 2}" +
                                " h=${(b?.bottom ?: 0) - (b?.top ?: 0)}" +
                                " x=${b?.left}..${b?.right}" +
                                " angle=${"%.1f".format(line.angle)}" +
                                " | ${line.text}",
                        )
                    }
            }
        } finally {
            recogniser.close()
        }
    }

    private companion object {
        val FORMATS = setOf("jpg", "jpeg", "png", "webp", "heic")
    }
}
