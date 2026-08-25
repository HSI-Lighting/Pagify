# Privacy Policy for Pagify

**Last Updated:** August 25, 2026

## 1. Introduction

Pagify is a privacy-focused PDF viewer application designed to process documents locally on your device. This Privacy Policy explains how we handle your data and respect your privacy.

## 2. Data Collection & Processing

### What We Do NOT Collect
- We do **not** collect personal information such as your name, email, phone number, or location
- We do **not** send your PDF files to external servers
- We do **not** track your usage behavior or app interactions
- We do **not** store or share your documents with third parties

### What Happens Locally on Your Device
Pagify processes all PDF operations exclusively on your device:
- **PDF Viewing:** All rendering and display happens locally
- **Text Extraction:** Text is extracted and processed on-device only
- **OCR Processing:** Optical Character Recognition (using Google ML Kit) runs entirely on your device
- **Extracted Text:** Any text you extract is stored only in your app's local cache

## 3. Third-Party Services

### Google ML Kit (On-Device OCR)
- Pagify uses Google's ML Kit for on-device text recognition (OCR)
- **No data is sent to Google servers** - All processing happens locally on your device
- ML Kit operates in offline mode and does not require internet connectivity
- For more details, see [Google ML Kit Privacy](https://developers.google.com/ml-kit/terms)

### Android System Services
- Pagify uses standard Android APIs for file access and storage
- Your documents are stored only where you choose to keep them on your device

## 4. Permissions

Pagify requests the following permissions:

| Permission | Purpose |
|-----------|---------|
| `READ_EXTERNAL_STORAGE` | Access PDF files you select |
| `WRITE_EXTERNAL_STORAGE` | Save extracted text or annotations |
| `INTERNET` | Optional - only if you share documents externally |

**You have full control:** Android allows you to grant or deny these permissions individually.

## 5. Data Retention

- **PDF Files:** Not stored by Pagify (stored in your device's file system where you placed them)
- **Extracted Text:** Cached only during your session; deleted when you close the app
- **Annotations/Notes:** Stored locally in your app cache; never transmitted
- **No Backups:** Pagify does not back up your data to cloud services

## 6. Security

- All document processing happens on your device, reducing exposure to network attacks
- Your files are never transmitted over the internet
- Pagify does not implement its own encryption; your device's file system provides security

## 7. Children's Privacy

Pagify is not intended for children under 13. We do not knowingly collect information from children. If you believe we have collected information from a child under 13, please contact us immediately.

## 8. Changes to This Policy

We may update this Privacy Policy occasionally. Changes will be reflected with an updated "Last Updated" date at the top of this policy. Continued use of Pagify after changes constitute acceptance of the updated policy.

## 9. Contact Us

If you have questions about this Privacy Policy or our privacy practices:

**Email:** dev@hsilighting.com  
**GitHub:** https://github.com/HSI-Lighting/Pagify

---

## Summary

✅ **Your PDFs stay on your device**  
✅ **No personal data collection**  
✅ **No cloud storage**  
✅ **No tracking or analytics**  
✅ **On-device processing only**

Pagify is designed with privacy by default.
