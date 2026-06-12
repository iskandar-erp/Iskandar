//! Cuentas por cobrar: créditos (cobros) y anticipos.

use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::common::{EntidadId, Extra};

/// Aplicación de un crédito contra un cargo existente (una factura por
/// cobrar, por ejemplo).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AplicacionCargo {
    pub cargo_id: Option<EntidadId>,
    pub folio_cargo: Option<String>,
    pub importe: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuevoCredito {
    /// Concepto de CxC según el catálogo del ERP (pago, nota de
    /// crédito, etc.).
    pub concepto_id: EntidadId,
    pub cliente_id: EntidadId,
    pub fecha: Option<NaiveDateTime>,
    pub folio: Option<String>,
    pub descripcion: Option<String>,
    pub aplicaciones: Vec<AplicacionCargo>,
    /// Parámetros específicos del provider, p. ej. `FORMA_COBRO_ID` o
    /// `USO_CFDI` en Microsip.
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credito {
    pub id: EntidadId,
    pub folio: String,
}
