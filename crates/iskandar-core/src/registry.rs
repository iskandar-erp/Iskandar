//! Registro de providers: de un nombre en la configuración a una
//! instancia viva.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::ProviderConfig;
use crate::error::{IskandarError, Result};
use crate::provider::ERPProvider;

/// Fábrica que construye un provider a partir de su sección de
/// configuración.
pub type ProviderFactory =
    Box<dyn Fn(&ProviderConfig) -> Result<Arc<dyn ERPProvider>> + Send + Sync>;

#[derive(Default)]
pub struct ProviderRegistry {
    factories: HashMap<String, ProviderFactory>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, factory: ProviderFactory) {
        self.factories.insert(name.into(), factory);
    }

    pub fn create(&self, name: &str, config: &ProviderConfig) -> Result<Arc<dyn ERPProvider>> {
        let factory = self
            .factories
            .get(name)
            .ok_or_else(|| IskandarError::UnknownProvider(name.to_string()))?;
        factory(config)
    }

    /// Providers compilados en este binario.
    pub fn names(&self) -> Vec<&str> {
        self.factories.keys().map(String::as_str).collect()
    }
}
