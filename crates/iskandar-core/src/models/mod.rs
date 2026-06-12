//! Modelos universales compartidos entre providers.
//!
//! Convención de idioma: los sustantivos de dominio van en español
//! (Cliente, Factura, Renglon, Poliza) porque es el vocabulario real de
//! los ERPs de la región — CFDI, folio o póliza no se traducen bien.
//! La infraestructura (ProviderConfig, ProviderRegistry) va en inglés.
//!
//! Tres reglas de diseño pensadas en escala continental:
//!
//! 1. **Dinero siempre en [`rust_decimal::Decimal`]**, nunca `f64`.
//! 2. **IDs flexibles** ([`EntidadId`]): los ERPs de escritorio usan
//!    enteros, los cloud usan UUIDs/strings. El core soporta ambos.
//! 3. **Campo `extra` en todo documento**: cada ERP tiene campos que no
//!    mapean al modelo universal (USO_CFDI en México, resolución DIAN en
//!    Colombia). Van ahí, sin romper el contrato.

mod clientes;
mod common;
mod compras;
mod contabilidad;
mod cxc;
mod inventario;
mod ventas;

pub use clientes::*;
pub use common::*;
pub use compras::*;
pub use contabilidad::*;
pub use cxc::*;
pub use inventario::*;
pub use ventas::*;
