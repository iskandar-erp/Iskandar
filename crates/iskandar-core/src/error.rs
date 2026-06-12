//! Error universal del framework.
//!
//! Convención del proyecto: `Result<T, E>` siempre, sin `unwrap()` en
//! código de producción.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, IskandarError>;

#[derive(Debug, Error)]
pub enum IskandarError {
    #[error("el provider '{0}' no está registrado")]
    UnknownProvider(String),

    #[error("el provider '{provider}' no soporta el módulo '{module}'")]
    UnsupportedModule { provider: String, module: String },

    #[error("operación aún no implementada: {0}")]
    NotImplemented(&'static str),

    #[error("error de configuración: {0}")]
    Config(String),

    #[error("error de conexión: {0}")]
    Connection(String),

    #[error("error de validación: {0}")]
    Validation(String),

    #[error("no encontrado: {0}")]
    NotFound(String),

    /// Error nativo reportado por el ERP subyacente.
    ///
    /// `code` es el código tal como lo regresa el sistema (en Microsip,
    /// el retorno de cada función de la DLL; 0 = éxito) y `message` el
    /// texto recuperado del propio ERP (`GetLastErrorMessage`).
    #[error("error del ERP (código {code}): {message}")]
    Provider { code: i32, message: String },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
