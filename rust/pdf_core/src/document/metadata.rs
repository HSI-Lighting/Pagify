//! Document-level metadata, serialised to JSON for the trip across JNI.
//!
//! JSON rather than a bespoke Java object keeps the JNI surface minimal (see the
//! "avoid JNI reflection" note in the architecture doc) at the cost of one small
//! parse on the Kotlin side, which happens once per document open.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    /// Raw PDF date strings (`D:20240131120000+01'00'`). Left unparsed on purpose:
    /// the Kotlin layer formats dates using the device locale.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modification_date: Option<String>,
    pub page_count: usize,
}

impl DocumentMetadata {
    /// PDFium hands back empty strings for absent tags; treat those as absent so
    /// the UI can fall back to the file name instead of rendering a blank title.
    pub fn set_tag(&mut self, name: &str, value: &str) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        let slot = match name {
            "Title" => &mut self.title,
            "Author" => &mut self.author,
            "Subject" => &mut self.subject,
            "Keywords" => &mut self.keywords,
            "Creator" => &mut self.creator,
            "Producer" => &mut self.producer,
            "CreationDate" => &mut self.creation_date,
            "ModificationDate" => &mut self.modification_date,
            _ => return,
        };
        *slot = Some(value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_tags_are_dropped_rather_than_stored() {
        let mut meta = DocumentMetadata::default();
        meta.set_tag("Title", "   ");
        meta.set_tag("Author", "");
        assert_eq!(meta.title, None);
        assert_eq!(meta.author, None);
    }

    #[test]
    fn tags_are_trimmed_and_unknown_tags_ignored() {
        let mut meta = DocumentMetadata::default();
        meta.set_tag("Title", "  Quarterly Report  ");
        meta.set_tag("NotARealTag", "value");
        assert_eq!(meta.title.as_deref(), Some("Quarterly Report"));
    }

    #[test]
    fn absent_fields_are_omitted_from_the_json_sent_to_kotlin() {
        let mut meta = DocumentMetadata {
            page_count: 12,
            ..Default::default()
        };
        meta.set_tag("Author", "A. Author");

        let json = serde_json::to_string(&meta).unwrap();
        assert_eq!(json, r#"{"author":"A. Author","pageCount":12}"#);
    }

    #[test]
    fn metadata_round_trips_through_json() {
        let mut meta = DocumentMetadata {
            page_count: 3,
            ..Default::default()
        };
        meta.set_tag("Title", "T");
        meta.set_tag("CreationDate", "D:20240131120000+01'00'");

        let json = serde_json::to_string(&meta).unwrap();
        let back: DocumentMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back, meta);
    }
}
