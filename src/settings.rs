//! The app's own settings, held in the shared registry (CONTRACT.md §2).
//!
//! Everything here is a **base** setting: the defaults a new document starts from and the styling a
//! document inherits when it says nothing of its own. Per-document overrides live in the document
//! (see [`crate::docstyle`]) — that split is the whole point, and it is the same split the browser's
//! editor uses, so a document styled in one opens correctly styled in the other.
//!
//! The same keys are declared in `packaging/settings/com.webgen.Word.toml`, so System Settings
//! renders this panel generically without Word writing any GTK for it (CONTRACT.md §3).

use std::rc::Rc;

use webgen_registry::Registry;

pub const APP_ID: &str = "com.webgen.Word";

/// A handle on the registry that degrades quietly. If the store cannot be opened — a fresh account,
/// a read-only home — every read returns its default and every write is dropped. A word processor
/// that will not start because a settings database is missing would be a poor trade.
pub struct Settings {
    reg: Option<Rc<Registry>>,
}

impl Settings {
    pub fn open() -> Self {
        match Registry::open_default() {
            Ok(reg) => Settings { reg: Some(Rc::new(reg)) },
            Err(e) => {
                eprintln!("webgen-word: settings unavailable, using defaults ({e})");
                Settings { reg: None }
            }
        }
    }

    /// The handle the shared colour tool wants. `webgen-swatch` keeps its palettes in the same
    /// registry, so a colour saved in Paint or Swatch is offered here too.
    pub fn swatch_reg(&self) -> webgen_swatch::Reg {
        self.reg.clone()
    }

    /// A string setting. An empty stored value counts as unset, so clearing a row in System Settings
    /// restores the default rather than styling everything with the empty string.
    pub fn string(&self, key: &str, default: &str) -> String {
        self.reg
            .as_ref()
            .and_then(|r| r.get_string(APP_ID, key))
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| default.to_string())
    }

    pub fn i64(&self, key: &str, default: i64) -> i64 {
        self.reg.as_ref().map(|r| r.get_i64(APP_ID, key, default)).unwrap_or(default)
    }

    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.reg.as_ref().map(|r| r.get_bool(APP_ID, key, default)).unwrap_or(default)
    }

    pub fn set_string(&self, key: &str, value: &str) {
        if let Some(r) = self.reg.as_ref() {
            let _ = r.set_string(APP_ID, key, value);
        }
    }

    pub fn set_i64(&self, key: &str, value: i64) {
        if let Some(r) = self.reg.as_ref() {
            let _ = r.set_i64(APP_ID, key, value);
        }
    }
}
