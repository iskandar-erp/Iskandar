//! Compras: proveedores, órdenes de compra y compras.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::common::{EntidadId, Extra, IdFiscal, Renglon};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proveedor {
    pub id: EntidadId,
    pub nombre: String,
    pub id_fiscal: Option<IdFiscal>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuevaOrdenCompra {
    pub proveedor_id: EntidadId,
    pub almacen_id: Option<EntidadId>,
    pub fecha: Option<NaiveDate>,
    pub folio: Option<String>,
    pub fecha_entrega: Option<NaiveDate>,
    pub renglones: Vec<Renglon>,
    pub descripcion: Option<String>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrdenCompra {
    pub id: EntidadId,
    pub folio: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuevaCompra {
    pub proveedor_id: EntidadId,
    pub almacen_id: Option<EntidadId>,
    /// Número de factura del proveedor (obligatorio en varios ERPs,
    /// p. ej. Microsip).
    pub factura_proveedor: String,
    pub fecha: Option<NaiveDate>,
    pub folio: Option<String>,
    pub renglones: Vec<Renglon>,
    pub descripcion: Option<String>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compra {
    pub id: EntidadId,
    pub folio: String,
}
