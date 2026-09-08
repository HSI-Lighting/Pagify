package com.hsilighting.pagify.core

import android.graphics.Bitmap

/**
 * The door into the 3D engine.
 *
 * Separate from [NativeBridge] on purpose. Every symbol here has a matching
 * `#[no_mangle]` export in `rust/pdf_core/src/jni_bridge/step_bridge.rs`, and
 * the two registries on the Rust side are separate too — nothing in the model
 * viewer can reach a document session, so a fault in this feature cannot
 * disturb the reader anybody actually uses.
 *
 * The library is loaded by [NativeBridge]; touching it here is what makes sure
 * that has happened, whichever of the two is used first.
 */
internal object StepBridge {

    const val NO_MODEL: Long = 0L

    init {
        // Referring to it loads it. The engine is one library; this is a second
        // door into it, not a second library.
        NativeBridge.nativeVersion()
    }

    /**
     * Open a STEP file.
     *
     * Throws with a sentence meant for the screen — "this model has 7,981
     * faces", not an error code. Half of real files contain something that
     * cannot be shown, so the refusal is an ordinary outcome and has to be
     * readable.
     */
    @Throws(PdfException::class)
    external fun openModel(path: String): Long

    external fun closeModel(handle: Long): Boolean

    /** Triangle count, faces in the file, what was skipped, and the size. */
    external fun modelSummaryJson(handle: Long): String

    /** Draw into [bitmap], which must be ARGB_8888. */
    external fun renderModelInto(handle: Long, bitmap: Bitmap): Boolean

    /** Turn it. Both distances are fractions of the view, not pixels. */
    external fun orbitModel(handle: Long, across: Float, down: Float): Boolean

    external fun panModel(handle: Long, across: Float, down: Float): Boolean

    /** Above one is closer. */
    external fun zoomModel(handle: Long, by: Float): Boolean

    external fun fitModel(handle: Long): Boolean

    /** For a test to prove that closing a model actually releases it. */
    external fun openModelCount(): Long
}
