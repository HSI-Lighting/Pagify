package com.hsilighting.pagify.core

import android.content.Context
import android.net.Uri
import android.util.Log
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.text.TextRecognition
import com.google.mlkit.vision.text.latin.TextRecognizerOptions
import org.json.JSONArray
import org.json.JSONObject
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

/**
 * Reading a business card from a photograph.
 *
 * Two routes, tried in that order because they are not equally good.
 *
 * A growing share of cards carry a QR encoding a complete vCard, and when one
 * does the data is **exact** — no recognising the letters, no guessing which line
 * is the company. Nothing downstream can improve on it, so it is tried first and
 * accepted outright.
 *
 * Failing that, the card is read the way a person reads it: recognise the
 * printed text, then work out what the lines mean. That second half is the
 * engine's — [NativeBridge.parsePhotographedCard] — so a card read here and the
 * same card read on iOS give the same answer. What comes back is a set of
 * guesses with confidences attached, which is why nothing on this path should be
 * saved without somebody seeing it first.
 *
 * What is still missing is the step between: finding the card's edges in the
 * photograph and flattening its perspective. Without it the engine measures the
 * card from the extent of its own text, which holds up while the card roughly
 * fills the frame and degrades as it stops doing so.
 */
object CardScanner {

    private const val TAG = "CardScanner"

    /** Which route read the card. The two are not equally trustworthy. */
    enum class Source {
        /** Exact, from a vCard in a QR code. */
        QR,

        /** Recognised and inferred from the printed text. Wants checking. */
        PRINT,
    }

    /**
     * What a photograph turned out to hold.
     *
     * [NotAContact] survives the arrival of OCR rather than folding into
     * [NothingFound]: it means a QR was read and held a web address, and once the
     * printed text has also come to nothing, that address is the only thing the
     * photograph yielded. It is worth offering.
     */
    sealed interface Outcome {
        /**
         * @param qrPayload a QR that was not a vCard, when the card was read from
         *   its printed text instead. The website is usually printed on the card
         *   as well, but not always — and this is the copy that cannot have been
         *   misread.
         */
        data class Contact(
            val card: String,
            val source: Source,
            val qrPayload: String? = null,
        ) : Outcome

        data class NotAContact(val payload: String) : Outcome
        data object NothingFound : Outcome
        data class Failed(val reason: String) : Outcome
    }

    /** Read a card: its QR if it has one, its printed text if it does not. */
    suspend fun read(context: Context, image: Uri): Outcome {
        val input = try {
            InputImage.fromFilePath(context, image)
        } catch (t: Throwable) {
            Log.e(TAG, "could not open the image", t)
            return Outcome.Failed(t.message ?: "That image could not be opened.")
        }

        val barcodes = try {
            scan(input)
        } catch (t: Throwable) {
            // Not fatal any more. A barcode detector that fell over says nothing
            // about whether the card can be read by eye, so this goes on to try.
            Log.w(TAG, "the barcode scan failed; reading the printed text instead", t)
            emptyList()
        }

        // Every payload is offered to the engine, not just the first: a card can
        // carry two codes — one for the vCard and one for a website — and which
        // is which is not knowable from the barcode alone.
        for (payload in barcodes) {
            val card = runCatching { NativeBridge.contactFromVCard(payload) }
                .onFailure { Log.w(TAG, "could not read a payload as a contact", it) }
                .getOrNull()
            if (card != null) return Outcome.Contact(card, Source.QR)
        }

        val printed = try {
            readPrintedText(input)
        } catch (t: Throwable) {
            Log.e(TAG, "the text recognition failed", t)
            return Outcome.Failed(t.message ?: "The card could not be read.")
        }

        // The engine returns a card either way; an empty one is not worth saving.
        // Judged on the fields rather than on whether any text was recognised at
        // all, because a photograph of a page of notes recognises plenty and
        // yields no contact.
        if (printed != null && worthKeeping(printed)) {
            return Outcome.Contact(printed, Source.PRINT, barcodes.firstOrNull())
        }

        barcodes.firstOrNull()?.let { return Outcome.NotAContact(it) }
        return Outcome.NothingFound
    }

    /**
     * Recognise the printed text and let the engine read the fields off it.
     *
     * The boxes go across in the photograph's own pixel space, unscaled: the
     * engine's rules are about relative position and relative text size, so the
     * units cancel and there is nothing to convert into.
     */
    private suspend fun readPrintedText(image: InputImage): String? {
        val segments = recogniseLines(image)
        if (segments.isEmpty()) return null

        val json = JSONArray().apply { segments.forEach { put(it) } }.toString()

        return runCatching { NativeBridge.parsePhotographedCard(json) }
            .onFailure { Log.e(TAG, "the engine could not read the card", it) }
            .getOrNull()
    }

    /**
     * ML Kit's text recognition, as a suspending call returning line boxes.
     *
     * Lines rather than words or blocks. The engine groups whatever it is given
     * into rows anyway, but a line is the unit ML Kit is most confident about,
     * and a block can span a whole column — which would hand the parser one
     * segment holding the name, the title and the company at once.
     *
     * Latin-only, matching [PageTextRecogniser]. A bilingual card gives up its
     * Latin half and loses the other, which is worth fixing but is a second model
     * to download rather than a change here.
     */
    private suspend fun recogniseLines(image: InputImage): List<JSONObject> =
        suspendCancellableCoroutine { continuation ->
            val recogniser = TextRecognition.getClient(TextRecognizerOptions.DEFAULT_OPTIONS)
            recogniser.process(image)
                .addOnSuccessListener { recognised ->
                    val lines = buildList {
                        for (block in recognised.textBlocks) {
                            for (line in block.lines) {
                                val box = line.boundingBox ?: continue
                                if (line.text.isBlank()) continue
                                add(
                                    segmentJson(
                                        box.left,
                                        box.top,
                                        box.right,
                                        box.bottom,
                                        line.text,
                                    ),
                                )
                            }
                        }
                    }
                    continuation.resume(lines)
                }
                .addOnFailureListener { continuation.resume(emptyList()) }
                // Closed on every path, including cancellation: these hold native
                // detectors, and leaking one per photograph would add up.
                .addOnCompleteListener { runCatching { recogniser.close() } }

            continuation.invokeOnCancellation { runCatching { recogniser.close() } }
        }

    /**
     * One recognised line in the shape `contacts::parse::TextSegment` decodes
     * from.
     *
     * Its own function so the field names can be tested without a device or a
     * photograph. Get one wrong and serde rejects the whole array, so *every*
     * scan fails identically — which looks like recognition not working rather
     * than a misspelled key.
     *
     * Takes plain integers rather than an `android.graphics.Rect` for the same
     * reason [PageTextRecogniser.segmentFor] does: the stubbed `Rect` in the
     * unit-test `android.jar` reads back as all zeroes without failing, which
     * quietly turns every assertion about position into 0 against 0.
     */
    internal fun segmentJson(
        left: Int,
        top: Int,
        right: Int,
        bottom: Int,
        text: String,
    ): JSONObject = JSONObject().apply {
        put("left", left)
        put("top", top)
        put("right", right)
        put("bottom", bottom)
        put("text", text)
    }

    /**
     * Whether a parsed card is worth offering to somebody.
     *
     * A name, a company, or a way of reaching them. Notes and raw text do not
     * count — every photograph with any writing in it produces those, and a
     * contact holding nothing but raw text is a blank row in the list.
     */
    private fun worthKeeping(cardJson: String): Boolean {
        val contact = runCatching { contactFromCardJson(cardJson, 0L) }.getOrNull() ?: return false
        return contact.name.isNotBlank() ||
            contact.company.isNotBlank() ||
            contact.phones.isNotEmpty() ||
            contact.emails.isNotEmpty()
    }

    /** ML Kit's callback API, as a suspending call. */
    private suspend fun scan(image: InputImage): List<String> =
        suspendCancellableCoroutine { continuation ->
            val scanner = BarcodeScanning.getClient()
            scanner.process(image)
                .addOnSuccessListener { found ->
                    continuation.resume(
                        found.mapNotNull { barcode ->
                            // rawValue rather than displayValue: a vCard's
                            // display form is a summary ML Kit built for showing
                            // somebody, and the engine needs the whole payload.
                            barcode.rawValue?.takeIf { it.isNotBlank() }
                        },
                    )
                }
                .addOnFailureListener { continuation.resume(emptyList()) }
                .addOnCompleteListener { runCatching { scanner.close() } }

            continuation.invokeOnCancellation { runCatching { scanner.close() } }
        }

    /** Unused for now, but names why `Barcode` is imported. */
    @Suppress("unused")
    private fun isLikelyContact(barcode: Barcode): Boolean =
        barcode.valueType == Barcode.TYPE_CONTACT_INFO
}
