//! # iskandar-core
//!
//! El contrato universal de Iskandar: el trait [`ERPProvider`] y los
//! modelos compartidos que cualquier ERP de América Latina implementa.
//!
//! Este crate no sabe de DLLs, ni de Firebird, ni de HTTP. Solo define
//! el lenguaje común. Los providers (crates `iskandar-*` bajo
//! `crates/providers/`) traducen ese lenguaje al dialecto de cada ERP.

pub mod client;
pub mod config;
pub mod error;
pub mod models;
pub mod provider;
pub mod registry;
pub mod security;

pub use client::ERPClient;
pub use config::ProviderConfig;
pub use error::{IskandarError, Result};
pub use provider::{
    Capacidades, ClientesModule, ComprasModule, ContabilidadModule, CxcModule, ERPProvider,
    FacturasModule, InventarioModule, PedidosModule,
};
pub use registry::{ProviderFactory, ProviderRegistry};
pub use security::{
    AuditError, AuditReport, Disposition, Finding, FindingId, GateOutcome, Remediation, Reverify,
    SecurityAudit, Severity,
};
