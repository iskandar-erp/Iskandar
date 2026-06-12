//! Configuración y tipos propios del provider Microsip.

use serde::{Deserialize, Serialize};

/// Sección `[providers.microsip]` del TOML del usuario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicrosipConfig {
    /// Ruta a `ApiMicrosip.dll`.
    pub dll_path: String,
    /// Base de empresa Firebird, local (`C:\...\SF.FDB`) o remota
    /// (`host:C:\...\SF.FDB`).
    pub db_path: String,
    pub usuario: String,
    pub password: String,
    /// Base de Metadatos, necesaria para `ChecaCompatibilidad*`.
    pub metadatos_path: Option<String>,
    /// Regla de negocio: permitir existencias negativas
    /// (`SetReglasVentas` / `SetReglasInventarios`).
    #[serde(default)]
    pub existencias_negativas: bool,
    /// Regla de negocio: validar precio mínimo en ventas.
    #[serde(default)]
    pub validar_precio_minimo: bool,
}
