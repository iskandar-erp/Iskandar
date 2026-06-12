//! # iskandar-microsip
//!
//! Provider para Microsip, ERP mexicano muy extendido en PyMEs de
//! México y Centroamérica. La integración es vía `ApiMicrosip.dll`
//! (sin API pública), que internamente conecta a Firebird 3.0.
//!
//! Solo compila la integración real en Windows — la DLL es Win32. En
//! otras plataformas el crate expone únicamente la configuración, para
//! que el workspace siga compilando en CI multiplataforma.

pub mod models;

#[cfg(windows)]
pub mod dll;
#[cfg(windows)]
pub mod provider;

pub use models::MicrosipConfig;
#[cfg(windows)]
pub use provider::MicrosipProvider;
