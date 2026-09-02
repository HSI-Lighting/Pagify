package com.hsilighting.pagify.core

import android.content.Context
import android.net.Uri
import android.util.Log
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

/**
 * Reading a business card from a photograph.
 *
 * Only the **QR path** so far, and that is deliberate rather than partial. A
 * growing share of cards carry a QR that encodes a complete vCard, and when one
 * does the data is *exact* — no detection of where the card is, no correcting its
 * perspective, no recognising the letters, no guessing which line is the company.
 * It is the shortest and most accurate route a card can take, so it is worth
 * having on its own before any of the rest exists.
 *
 * When there is no QR, or the QR holds a web address rather than a contact,
 * [read] returns null and the caller falls back to the reader — which for now
 * means telling somebody so, rather than pretending to have read the card.
 */
object CardScanner {

    private const val TAG = "CardScanner"

    /**
     * What a photograph turned out to hold.
     *
     * The three outcomes are distinct on purpose. "No QR at all" and "a QR that
     * was not a contact" look the same to the user but mean different things to
     * whoever extends this later: the first needs OCR, the second needs OCR *and*
     * has a URL worth keeping.
     */
    sealed interface Outcome {
        data class Contact(val card: String) : Outcome
        data class NotAContact(val payload: String) : Outcome
        data object NothingFound : Outcome
        data class Failed(val reason: String) : Outcome
    }

    /**
     * Look for a vCard in the QR codes of an image.
     *
     * Returns the engine's card JSON when one is found. Reading the payload is
     * the engine's job, not ML Kit's: what counts as a vCard, and how forgiving
     * to be about a sloppy one, is the same question on every platform.
     */
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
            Log.e(TAG, "the scan failed", t)
            return Outcome.Failed(t.message ?: "The card could not be scanned.")
        }

        if (barcodes.isEmpty()) return Outcome.NothingFound

        // Every payload is offered to the engine, not just the first: a card can
        // carry two codes — one for the vCard and one for a website — and which
        // is which is not knowable from the barcode alone.
        for (payload in barcodes) {
            val card = runCatching { NativeBridge.contactFromVCard(payload) }
                .onFailure { Log.w(TAG, "could not read a payload as a contact", it) }
                .getOrNull()
            if (card != null) return Outcome.Contact(card)
        }
        return Outcome.NotAContact(barcodes.first())
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
                // Closed on every path, including cancellation: the scanner holds
                // a native detector, and leaking one per photograph would add up.
                .addOnCompleteListener { runCatching { scanner.close() } }

            continuation.invokeOnCancellation { runCatching { scanner.close() } }
        }

    /** Unused for now, but names why `Barcode` is imported. */
    @Suppress("unused")
    private fun isLikelyContact(barcode: Barcode): Boolean =
        barcode.valueType == Barcode.TYPE_CONTACT_INFO
}
