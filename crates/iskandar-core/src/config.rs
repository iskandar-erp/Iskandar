//! Configuración cruda de providers.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{IskandarError, Result};

/// Configuración sin tipar de un provider — el contenido de la sección
/// `[providers.<nombre>]` del TOML del usuario.
///
/// El core no conoce los campos de cada ERP; cada provider la convierte
/// a su propia config tipada con [`ProviderConfig::typed`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderConfig(pub serde_json::Map<String, serde_json::Value>);

impl ProviderConfig {
    pub fn typed<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(serde_json::Value::Object(self.0.clone()))
            .map_err(|e| IskandarError::Config(e.to_string()))
    }
}
