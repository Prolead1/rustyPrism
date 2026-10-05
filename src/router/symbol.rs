//! Symbol interning.
//!
//! `String`-keyed hash maps are convenient at the API boundary but expensive on
//! the hot path: every venue lookup would hash a string. Instead symbol names
//! are interned **once per order** into a dense [`SymbolId`], after which all
//! venue/book lookups are array indexing with no hashing.

use std::collections::HashMap;

/// Dense identifier for an interned symbol.
pub type SymbolId = u32;

/// Maps symbol names to dense ids and back.
#[derive(Debug, Default, Clone)]
pub struct SymbolRegistry {
    names: Vec<String>,
    ids: HashMap<String, SymbolId>,
}

impl SymbolRegistry {
    pub fn new() -> Self {
        SymbolRegistry::default()
    }

    /// Return the id for `name`, allocating a new one if needed.
    pub fn intern(&mut self, name: &str) -> SymbolId {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = self.names.len() as SymbolId;
        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);
        id
    }

    /// Look up an existing id without allocating.
    pub fn id(&self, name: &str) -> Option<SymbolId> {
        self.ids.get(name).copied()
    }

    /// Resolve an id back to its name.
    pub fn name(&self, id: SymbolId) -> Option<&str> {
        self.names.get(id as usize).map(|name| name.as_str())
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_is_stable() {
        let mut registry = SymbolRegistry::new();
        let a = registry.intern("AAPL");
        let b = registry.intern("MSFT");
        assert_ne!(a, b);
        assert_eq!(registry.intern("AAPL"), a);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
    }

    #[test]
    fn test_lookup_without_allocating() {
        let mut registry = SymbolRegistry::new();
        registry.intern("AAPL");
        assert_eq!(registry.id("AAPL"), Some(0));
        assert_eq!(registry.id("NOPE"), None);
        assert_eq!(registry.name(0), Some("AAPL"));
        assert_eq!(registry.name(99), None);
    }
}
