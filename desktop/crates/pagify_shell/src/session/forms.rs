//! Form fields: reading what an AcroForm holds.
//!
//! Part of the `session` module — split out of the single file the design
//! review flagged (Phase 4, file splits). Methods keep living on `Session`.

use super::*;

impl Session {
    /// Every form widget in the document, page by page, in page order — the
    /// same widget appearing on two pages is two entries, as a reader walks
    /// them.
    pub fn form_fields(&self) -> Result<Vec<pdf_core::document::FormFieldInfo>> {
        registry::with_session(self.handle, |s| s.document.form_fields())
    }

    /// Set a text field's value; `false` when the document holds no field by
    /// that name.
    pub fn set_form_text(&self, name: &str, value: &str) -> Result<bool> {
        registry::with_session(self.handle, |s| s.document.set_form_text(name, value))
    }

    /// Check or uncheck a checkbox; `false` when the document holds no field
    /// by that name.
    pub fn set_form_checkbox(&self, name: &str, checked: bool) -> Result<bool> {
        registry::with_session(self.handle, |s| s.document.set_form_checkbox(name, checked))
    }

    /// Check the radio widget carrying `state` in the group `name`; `false`
    /// when no widget matches.
    pub fn set_form_radio(&self, name: &str, state: &str) -> Result<bool> {
        registry::with_session(self.handle, |s| s.document.set_form_radio(name, state))
    }

    /// Every field as JSON — the shape `formexport` writes and `formimport`
    /// reads: a list of `{ "name", "kind", "value" }`, value `null` where the
    /// field holds nothing.
    pub fn form_data_json(&self) -> Result<String> {
        let fields = self.form_fields()?;
        let entries: Vec<serde_json::Value> = fields
            .iter()
            .map(|f| {
                serde_json::json!({
                    "name": f.name,
                    "kind": match f.kind {
                        pdf_core::document::FormFieldKind::Text => "text",
                        pdf_core::document::FormFieldKind::Checkbox => "checkbox",
                        pdf_core::document::FormFieldKind::Radio => "radio",
                        pdf_core::document::FormFieldKind::Combo => "combo",
                        pdf_core::document::FormFieldKind::List => "list",
                        pdf_core::document::FormFieldKind::Button => "button",
                        pdf_core::document::FormFieldKind::Signature => "signature",
                        pdf_core::document::FormFieldKind::Unknown => "unknown",
                    },
                    "value": f.value,
                })
            })
            .collect();
        Ok(serde_json::Value::Array(entries).to_string())
    }

    /// Apply the JSON `formexport` writes.
    ///
    /// Text values, checkbox states and radio selections are settable in
    /// this build; anything else is counted as not applied rather than
    /// guessed at. A checkbox or radio entry is checked/selected when its
    /// value is present, unchecked when it is `null`; a `null` radio entry
    /// is left alone, because a radio group is a choice, not a switch.
    ///
    /// Returns `(applied, not_applied)`.
    pub fn apply_form_data_json(&self, json: &str) -> Result<(usize, usize)> {
        let parsed: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| pdf_core::PdfError::InvalidArgument(format!("not form data: {e}")))?;
        let entries = parsed.as_array().ok_or_else(|| {
            pdf_core::PdfError::InvalidArgument("not form data: expected a list".into())
        })?;
        let mut applied = 0usize;
        let mut not_applied = 0usize;
        for entry in entries {
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let kind = entry.get("kind").and_then(|v| v.as_str()).unwrap_or("unknown");
            let value = entry.get("value").and_then(|v| v.as_str());
            let ok = if name.is_empty() {
                false
            } else {
                match kind {
                    "text" => self.set_form_text(name, value.unwrap_or(""))?,
                    "checkbox" => self.set_form_checkbox(name, value.is_some())?,
                    "radio" => match value {
                        Some(state) => self.set_form_radio(name, state)?,
                        None => true,
                    },
                    _ => false,
                }
            };
            if ok {
                applied += 1;
            } else {
                not_applied += 1;
            }
        }
        Ok((applied, not_applied))
    }
}
