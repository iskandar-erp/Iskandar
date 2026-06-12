//! Documentos de venta: facturas y pedidos.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::common::{EntidadId, Extra, Moneda, Renglon};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuevaFactura {
    pub cliente_id: EntidadId,
    /// `None` = fecha del día según el ERP.
    pub fecha: Option<NaiveDate>,
    /// `None` = folio automático (serie por defecto del ERP).
    pub folio: Option<String>,
    pub renglones: Vec<Renglon>,
    pub moneda: Option<Moneda>,
    pub descripcion: Option<String>,
    /// Parámetros específicos del provider, p. ej. `USO_CFDI` o
    /// `LUGAR_EXPEDICION_ID` en México.
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Factura {
    pub id: EntidadId,
    pub folio: String,
    pub fecha: NaiveDate,
    pub cliente_id: EntidadId,
    pub total: Option<Decimal>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuevoPedido {
    pub cliente_id: EntidadId,
    pub fecha: Option<NaiveDate>,
    pub folio: Option<String>,
    pub fecha_entrega: Option<NaiveDate>,
    pub renglones: Vec<Renglon>,
    pub descripcion: Option<String>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pedido {
    pub id: EntidadId,
    pub folio: String,
    pub fecha: NaiveDate,
    pub cliente_id: EntidadId,
    #[serde(default)]
    pub extra: Extra,
}
