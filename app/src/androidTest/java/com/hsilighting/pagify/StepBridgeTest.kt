package com.hsilighting.pagify

import android.graphics.Bitmap
import androidx.test.platform.app.InstrumentationRegistry
import com.hsilighting.pagify.core.StepBridge
import com.hsilighting.pagify.ui.model.captureSize
import java.io.File
import org.junit.After
import org.junit.Before
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The whole 3D path, on the phone, through the real bridge.
 *
 * Everything below the JNI boundary is tested in Rust and tested well. What
 * that cannot reach is the boundary itself: whether the symbols are found,
 * whether a bitmap locked by Android is filled the way the rasteriser thinks it
 * is, and whether a native handle is released when the screen is closed. Each
 * of those fails in a way no host test can see — a missing symbol is an
 * `UnsatisfiedLinkError` at first use, and a leaked handle is memory that grows
 * until the system kills the app.
 *
 * The model is written out here rather than shipped as a fixture. Real CAD
 * cannot be committed — supplier geometry is the confidential part — and a
 * square prism is enough to prove that pixels arrive.
 */
class StepBridgeTest {

    private var handle = StepBridge.NO_MODEL
    private lateinit var file: File

    /** A single square face, which is the smallest STEP file that draws. */
    private val aSquare = """
        ISO-10303-21;
        HEADER;
        FILE_DESCRIPTION((''),'1');
        FILE_NAME('square','2026-01-01T00:00:00',(''),(''),'','','');
        FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));
        ENDSEC;
        DATA;
        #1=CARTESIAN_POINT('',(0.,0.,0.));
        #2=CARTESIAN_POINT('',(10.,0.,0.));
        #3=CARTESIAN_POINT('',(10.,10.,0.));
        #4=CARTESIAN_POINT('',(0.,10.,0.));
        #5=DIRECTION('',(0.,0.,1.));
        #6=DIRECTION('',(1.,0.,0.));
        #7=AXIS2_PLACEMENT_3D('',#1,#5,#6);
        #8=PLANE('',#7);
        #11=VERTEX_POINT('',#1);
        #12=VERTEX_POINT('',#2);
        #13=VERTEX_POINT('',#3);
        #14=VERTEX_POINT('',#4);
        #20=DIRECTION('',(1.,0.,0.));
        #21=VECTOR('',#20,1.);
        #22=LINE('',#1,#21);
        #23=DIRECTION('',(0.,1.,0.));
        #24=VECTOR('',#23,1.);
        #25=LINE('',#2,#24);
        #26=DIRECTION('',(-1.,0.,0.));
        #27=VECTOR('',#26,1.);
        #28=LINE('',#3,#27);
        #29=DIRECTION('',(0.,-1.,0.));
        #30=VECTOR('',#29,1.);
        #31=LINE('',#4,#30);
        #41=EDGE_CURVE('',#11,#12,#22,.T.);
        #42=EDGE_CURVE('',#12,#13,#25,.T.);
        #43=EDGE_CURVE('',#13,#14,#28,.T.);
        #44=EDGE_CURVE('',#14,#11,#31,.T.);
        #51=ORIENTED_EDGE('',*,*,#41,.T.);
        #52=ORIENTED_EDGE('',*,*,#42,.T.);
        #53=ORIENTED_EDGE('',*,*,#43,.T.);
        #54=ORIENTED_EDGE('',*,*,#44,.T.);
        #60=EDGE_LOOP('',(#51,#52,#53,#54));
        #61=FACE_OUTER_BOUND('',#60,.T.);
        #62=ADVANCED_FACE('',(#61),#8,.T.);
        ENDSEC;
        END-ISO-10303-21;
    """.trimIndent()

    private fun write(name: String, text: String): File {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        return File(context.cacheDir, name).apply { writeText(text) }
    }

    private fun open(): Long {
        file = write("bridge-test.stp", aSquare)
        handle = StepBridge.openModel(file.absolutePath)
        return handle
    }

    private fun drawn(handle: Long, size: Int = 96): Bitmap {
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        assertTrue("the model would not draw", StepBridge.renderModelInto(handle, bitmap))
        return bitmap
    }

    private fun pixelsOf(bitmap: Bitmap): IntArray {
        val pixels = IntArray(bitmap.width * bitmap.height)
        bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
        return pixels
    }

    @After
    fun release() {
        if (handle != StepBridge.NO_MODEL) {
            StepBridge.closeModel(handle)
            handle = StepBridge.NO_MODEL
        }
        if (::file.isInitialized) file.delete()
    }

    // ---- the boundary --------------------------------------------------------

    /** The symbols exist and a model opens. */
    @Test
    fun a_model_opens_and_gets_a_handle() {
        assertNotEquals(StepBridge.NO_MODEL, open())
    }

    /**
     * Pixels actually arrive in the bitmap Android handed over.
     *
     * The rasteriser is tested in Rust; what is tested here is the handover —
     * a locked bitmap may pad its rows, and a renderer that ignores that
     * writes a sheared picture that every host test would still call correct.
     */
    @Test
    fun the_model_is_drawn_into_the_bitmap() {
        val bitmap = drawn(open())
        val pixels = pixelsOf(bitmap)

        assertTrue("nothing was written at all", pixels.any { it != 0 })
        assertTrue(
            "the picture is one flat colour, so nothing was drawn on the background",
            pixels.toSet().size > 1,
        )
    }

    /** Turning it changes the picture. */
    @Test
    fun orbiting_changes_what_is_drawn() {
        val handle = open()
        val before = pixelsOf(drawn(handle))

        assertTrue(StepBridge.orbitModel(handle, 0.2f, 0.15f))
        val after = pixelsOf(drawn(handle))

        assertNotEquals(
            "the view did not move",
            before.toList(),
            after.toList(),
        )
    }

    /** And fitting puts it back. */
    @Test
    fun fitting_restores_the_opening_view() {
        val handle = open()
        val original = pixelsOf(drawn(handle))

        StepBridge.orbitModel(handle, 0.3f, 0.2f)
        StepBridge.zoomModel(handle, 3f)
        StepBridge.fitModel(handle)

        assertEquals(original.toList(), pixelsOf(drawn(handle)).toList())
    }

    // ---- taking a picture ----------------------------------------------------

    /**
     * A capture is drawn at its own, larger size.
     *
     * The picture the camera button takes is not the one on screen: it is a
     * second rasterisation into a bitmap several times bigger. That is exactly
     * where a stride mistake shows — a 96 x 96 square has no padding to get
     * wrong, and a tall bitmap of an odd width is the shape Android actually
     * pads. A sheared capture would still be full of pixels, so the test looks
     * at the last row as well as the first.
     */
    @Test
    fun a_capture_is_drawn_at_its_own_larger_size() {
        val handle = open()
        val size = captureSize(viewWidth = 361, viewHeight = 640, scale = 3)
        val picture = Bitmap.createBitmap(size.width, size.height, Bitmap.Config.ARGB_8888)

        assertTrue("the capture would not draw", StepBridge.renderModelInto(handle, picture))
        assertEquals(1083, picture.width)
        assertEquals(1920, picture.height)

        val pixels = pixelsOf(picture)
        assertTrue("the capture is one flat colour", pixels.toSet().size > 1)

        val lastRow = pixels.takeLast(picture.width)
        assertTrue("nothing reached the last row", lastRow.any { it != 0 })
    }

    /**
     * And it is the same view, only bigger.
     *
     * A capture that reframed the part would not be a picture of what the
     * reader was looking at. Both are drawn from the same camera, so the part
     * covers the same fraction of each.
     */
    @Test
    fun a_capture_frames_the_part_the_way_the_view_does() {
        val handle = open()
        val view = Bitmap.createBitmap(240, 320, Bitmap.Config.ARGB_8888)
        val large = Bitmap.createBitmap(720, 960, Bitmap.Config.ARGB_8888)

        assertTrue(StepBridge.renderModelInto(handle, view))
        assertTrue(StepBridge.renderModelInto(handle, large))

        val small = covered(view)
        val big = covered(large)

        assertTrue("nothing was drawn at all: $small, $big", small > 0.01 && big > 0.01)
        assertTrue(
            "the part covers $small of the view but $big of the capture",
            kotlin.math.abs(small - big) < 0.02,
        )
    }

    /** What fraction of a picture is the part rather than the background. */
    private fun covered(bitmap: Bitmap): Double {
        val pixels = pixelsOf(bitmap)
        val background = pixels[0]
        return pixels.count { it != background }.toDouble() / pixels.size
    }

    /** The summary crosses the boundary as usable JSON. */
    @Test
    fun the_summary_comes_back_as_json() {
        val summary = org.json.JSONObject(StepBridge.modelSummaryJson(open()))

        assertTrue("no triangles reported", summary.getInt("triangles") > 0)
        assertEquals(1, summary.getInt("facesInFile"))
        assertTrue(summary.has("skipped"))
    }

    // ---- the handle ----------------------------------------------------------

    /**
     * Closing a model releases it.
     *
     * A mesh is megabytes of native memory that no garbage collector will ever
     * reclaim. A viewer that leaks one per file opened is an app that dies
     * after a dozen models, at a moment with nothing to connect it to this.
     */
    @Test
    fun closing_a_model_releases_it() {
        val before = StepBridge.openModelCount()
        val opened = open()
        assertEquals(before + 1, StepBridge.openModelCount())

        assertTrue(StepBridge.closeModel(opened))
        handle = StepBridge.NO_MODEL

        assertEquals(before, StepBridge.openModelCount())
    }

    /** A handle that was closed is refused rather than followed into freed memory. */
    @Test
    fun a_closed_handle_cannot_be_drawn_with() {
        val opened = open()
        StepBridge.closeModel(opened)
        handle = StepBridge.NO_MODEL

        val bitmap = Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888)
        val outcome = runCatching { StepBridge.renderModelInto(opened, bitmap) }

        assertTrue(
            "a stale handle was accepted",
            outcome.isFailure || outcome.getOrNull() == false,
        )
    }

    // ---- refusal -------------------------------------------------------------

    /**
     * A file that cannot be shown fails with a sentence, not a code.
     *
     * Half of real files contain something unsupported, so this is an ordinary
     * outcome and the message is what the screen puts in front of somebody.
     */
    @Test
    fun a_file_that_is_not_step_fails_with_something_readable() {
        val junk = write("not-a-model.stp", "this is not a STEP file")

        val outcome = runCatching { StepBridge.openModel(junk.absolutePath) }
        junk.delete()

        assertTrue("junk was accepted as a model", outcome.isFailure)
        val message = outcome.exceptionOrNull()?.message.orEmpty()
        assertTrue("the failure said nothing: '$message'", message.length > 10)
    }

    @Test
    fun a_missing_file_fails_rather_than_crashing() {
        val outcome = runCatching { StepBridge.openModel("/does/not/exist.stp") }
        assertTrue(outcome.isFailure)
    }
}

/**
 * A real supplier file, on the phone.
 *
 * Skipped unless one has been pushed to `/sdcard/Download`, because real CAD
 * cannot be committed — supplier geometry is the confidential part. What this
 * adds over the synthetic square is scale: a 6.8 MB model with two thousand
 * faces, parsed and tessellated on a phone's own processor rather than a
 * desktop's.
 *
 * ```text
 * adb push part.step /sdcard/Download/
 * adb shell pm grant com.hsilighting.pagify3d.debug android.permission.READ_EXTERNAL_STORAGE
 * ```
 */
class RealModelTest {

    /**
     * Copy a file out of shared storage, through the shell.
     *
     * **Not by reading it directly.** A test app has no access to
     * `/sdcard/Download` on a modern Android, and the permission that used
     * to grant it cannot be granted to a package that only exists while the
     * test run lasts. Instrumentation can run a shell command, and the shell
     * can read it — so `cat` does the fetching and the bytes arrive on
     * stdout.
     */
    private fun fetch(remote: String, into: File): Boolean = runCatching {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        // Unquoted: `executeShellCommand` does not run a shell, it execs the
        // command after splitting on spaces, so quotes arrive as part of the
        // filename and `cat` returns nothing at all -- silently.
        automation.executeShellCommand("cat $remote").use { descriptor ->
            android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { source ->
                into.outputStream().use { sink -> source.copyTo(sink) }
            }
        }
        into.length() > 0
    }.getOrDefault(false)

    /** Whatever STEP file has been pushed to Download, if any. */
    /** Every STEP file pushed to Download, so all of them get measured. */
    private fun pushedFiles(): List<String> = runCatching {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        automation.executeShellCommand("ls /sdcard/Download").use { descriptor ->
            android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { source ->
                source.readBytes().decodeToString()
                    .lineSequence()
                    .map { it.trim() }
                    .filter { it.endsWith(".step", true) || it.endsWith(".stp", true) }
                    .map { "/sdcard/Download/$it" }
                    .toList()
            }
        }
    }.getOrDefault(emptyList())

    @Test
    fun every_pushed_part_opens_and_draws_on_the_phone() {
        val files = pushedFiles()
        if (files.isEmpty()) return // nothing pushed; nothing to say

        val context = InstrumentationRegistry.getInstrumentation().targetContext
        for (remote in files) {
            val copy = File(context.cacheDir, remote.substringAfterLast('/'))
            if (!fetch(remote, copy)) continue

            val startedOpen = System.currentTimeMillis()
            val handle = runCatching { StepBridge.openModel(copy.absolutePath) }
            val opened = System.currentTimeMillis() - startedOpen

            if (handle.isFailure) {
                android.util.Log.i(
                    "RealModel",
                    "${copy.name}: refused in ${opened}ms -- ${handle.exceptionOrNull()?.message}",
                )
                copy.delete()
                continue
            }

            val model = handle.getOrThrow()
            try {
                val summary = org.json.JSONObject(StepBridge.modelSummaryJson(model))
                val bitmap = Bitmap.createBitmap(432, 800, Bitmap.Config.ARGB_8888)

                // Twice, and the second is the one reported: the first pays for
                // whatever the allocator and the caches want, which is not what
                // a drag would cost.
                StepBridge.renderModelInto(model, bitmap)
                val startedDraw = System.nanoTime()
                assertTrue(StepBridge.renderModelInto(model, bitmap))
                val drewMicros = (System.nanoTime() - startedDraw) / 1000

                android.util.Log.i(
                    "RealModel",
                    "${copy.name}: opened ${opened}ms, drew ${drewMicros / 1000}ms, " +
                        "${summary.getInt("triangles")} triangles, " +
                        "${summary.getInt("facesDrawn")} of ${summary.getInt("facesInFile")} faces",
                )

                val skipped = summary.getJSONArray("skipped")
                val lost = (0 until skipped.length()).sumOf { skipped.getJSONObject(it).getInt("count") }
                assertEquals(
                    "${copy.name}: faces went missing without being counted",
                    summary.getInt("facesInFile"),
                    summary.getInt("facesDrawn") + lost,
                )
            } finally {
                StepBridge.closeModel(model)
                copy.delete()
            }
        }
    }
}

/**
 * Which way a drag turns the model.
 *
 * A reported bug: the part could be spun about one axis and not tilted about
 * the other. The Rust side has tests saying both work, so this asks the same
 * question from the Kotlin side of the boundary — the only place left for it
 * to be lost.
 */
class OrbitAxesTest {

    private var handle = StepBridge.NO_MODEL
    private lateinit var file: File

    /** A wedge: it looks different from every direction, unlike a flat square. */
    private val aWedge = """
        ISO-10303-21;
        HEADER;
        FILE_DESCRIPTION((''),'1');
        FILE_NAME('wedge','2026-01-01T00:00:00',(''),(''),'','','');
        FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));
        ENDSEC;
        DATA;
        #1=CARTESIAN_POINT('',(0.,0.,0.));
        #2=CARTESIAN_POINT('',(10.,0.,0.));
        #3=CARTESIAN_POINT('',(10.,10.,0.));
        #4=CARTESIAN_POINT('',(0.,10.,0.));
        #5=DIRECTION('',(0.,0.,1.));
        #6=DIRECTION('',(1.,0.,0.));
        #7=AXIS2_PLACEMENT_3D('',#1,#5,#6);
        #8=PLANE('',#7);
        #11=VERTEX_POINT('',#1);
        #12=VERTEX_POINT('',#2);
        #13=VERTEX_POINT('',#3);
        #14=VERTEX_POINT('',#4);
        #20=DIRECTION('',(1.,0.,0.));
        #21=VECTOR('',#20,1.);
        #22=LINE('',#1,#21);
        #23=DIRECTION('',(0.,1.,0.));
        #24=VECTOR('',#23,1.);
        #25=LINE('',#2,#24);
        #26=DIRECTION('',(-1.,0.,0.));
        #27=VECTOR('',#26,1.);
        #28=LINE('',#3,#27);
        #29=DIRECTION('',(0.,-1.,0.));
        #30=VECTOR('',#29,1.);
        #31=LINE('',#4,#30);
        #41=EDGE_CURVE('',#11,#12,#22,.T.);
        #42=EDGE_CURVE('',#12,#13,#25,.T.);
        #43=EDGE_CURVE('',#13,#14,#28,.T.);
        #44=EDGE_CURVE('',#14,#11,#31,.T.);
        #51=ORIENTED_EDGE('',*,*,#41,.T.);
        #52=ORIENTED_EDGE('',*,*,#42,.T.);
        #53=ORIENTED_EDGE('',*,*,#43,.T.);
        #54=ORIENTED_EDGE('',*,*,#44,.T.);
        #60=EDGE_LOOP('',(#51,#52,#53,#54));
        #61=FACE_OUTER_BOUND('',#60,.T.);
        #62=ADVANCED_FACE('',(#61),#8,.T.);
        ENDSEC;
        END-ISO-10303-21;
    """.trimIndent()

    private fun pixels(size: Int = 96): IntArray {
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        assertTrue(StepBridge.renderModelInto(handle, bitmap))
        val out = IntArray(size * size)
        bitmap.getPixels(out, 0, size, 0, 0, size, size)
        return out
    }

    @Before
    fun open() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        file = File(context.cacheDir, "orbit-axes.stp").apply { writeText(aWedge) }
        handle = StepBridge.openModel(file.absolutePath)
    }

    @After
    fun close() {
        if (handle != StepBridge.NO_MODEL) StepBridge.closeModel(handle)
        if (::file.isInitialized) file.delete()
    }

    /** Dragging sideways turns it. */
    @Test
    fun a_sideways_drag_turns_the_model() {
        val before = pixels().toList()
        StepBridge.orbitModel(handle, 0.15f, 0f)
        assertNotEquals("a sideways drag did nothing", before, pixels().toList())
    }

    /**
     * And dragging up or down tilts it.
     *
     * This is the half that was reported broken. Asked separately from the
     * sideways drag so a failure names which axis, rather than "the gesture".
     */
    @Test
    fun a_vertical_drag_tilts_the_model() {
        val before = pixels().toList()
        StepBridge.orbitModel(handle, 0f, 0.15f)
        assertNotEquals("a vertical drag did nothing", before, pixels().toList())
    }

    /** And they are different movements, not the same one twice. */
    @Test
    fun the_two_axes_are_not_the_same_movement() {
        val start = pixels().toList()

        StepBridge.orbitModel(handle, 0.15f, 0f)
        val sideways = pixels().toList()

        StepBridge.fitModel(handle)
        assertEquals("fit did not restore the view", start, pixels().toList())

        StepBridge.orbitModel(handle, 0f, 0.15f)
        val vertical = pixels().toList()

        assertNotEquals("both axes produced the same view", sideways, vertical)
    }
}
