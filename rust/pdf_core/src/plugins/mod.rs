//! Plugin surface for features that should not need core changes (OCR,
//! text-to-speech, redaction). Roadmap phase 5 — the trait is declared now so
//! later features have an obvious place to attach.

use crate::document::Document;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq)]
pub enum PluginEvent {
    DocumentOpened { page_count: usize },
    PageRendered { page_index: usize, zoom: f32 },
    PageChanged { page_index: usize },
    DocumentClosing,
}

pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn init(&mut self, doc: &dyn Document) -> Result<()>;
    fn on_event(&mut self, event: PluginEvent) -> Result<()>;
}

/// Holds the registered plugins for one document session.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
    }

    /// Fan an event out to every plugin.
    ///
    /// One misbehaving plugin must not stop the others from seeing the event, nor
    /// break the core's own flow, so failures are logged and collected rather than
    /// propagated with `?`.
    pub fn dispatch(&mut self, event: PluginEvent) -> Vec<(String, String)> {
        let mut failures = Vec::new();
        for plugin in &mut self.plugins {
            if let Err(e) = plugin.on_event(event.clone()) {
                let name = plugin.name().to_string();
                log::warn!("plugin {name} failed to handle {event:?}: {e}");
                failures.push((name, e.to_string()));
            }
        }
        failures
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PdfError;

    struct CountingPlugin {
        name: String,
        seen: usize,
        fail: bool,
    }

    impl Plugin for CountingPlugin {
        fn name(&self) -> &str {
            &self.name
        }
        fn init(&mut self, _doc: &dyn Document) -> Result<()> {
            Ok(())
        }
        fn on_event(&mut self, _event: PluginEvent) -> Result<()> {
            self.seen += 1;
            if self.fail {
                return Err(PdfError::Unsupported("test plugin"));
            }
            Ok(())
        }
    }

    #[test]
    fn one_failing_plugin_does_not_stop_the_others() {
        let mut registry = PluginRegistry::default();
        registry.register(Box::new(CountingPlugin {
            name: "bad".into(),
            seen: 0,
            fail: true,
        }));
        registry.register(Box::new(CountingPlugin {
            name: "good".into(),
            seen: 0,
            fail: false,
        }));

        let failures = registry.dispatch(PluginEvent::PageChanged { page_index: 2 });

        assert_eq!(registry.len(), 2);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "bad");
    }

    #[test]
    fn dispatching_to_an_empty_registry_is_a_no_op() {
        let mut registry = PluginRegistry::default();
        assert!(registry.is_empty());
        assert!(registry.dispatch(PluginEvent::DocumentClosing).is_empty());
    }
}
