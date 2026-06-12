//! Inventario: artículos y movimientos (entradas / salidas).

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::common::{EntidadId, Extra};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Articulo {
    pub id: EntidadId,
    pub nombre: String,
    pub clave: Option<String>,
    pub precio_lista: Option<Decimal>,
    pub existencia: Option<Decimal>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FiltroArticulos {
    pub texto: Option<String>,
    pub limite: Option<u32>,
    pub pagina: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovimientoRenglon {
    pub articulo_id: EntidadId,
    pub unidades: Decimal,
    /// `None` = costo calculado por el ERP.
    pub costo_unitario: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuevaEntrada {
    pub almacen_id: EntidadId,
    pub fecha: Option<NaiveDate>,
    pub folio: Option<String>,
    pub descripcion: Option<String>,
    pub renglones: Vec<MovimientoRenglon>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuevaSalida {
    pub almacen_id: EntidadId,
    /// Para traspasos: almacén que recibe (el ERP genera la entrada
    /// correspondiente). `None` = salida simple.
    pub almacen_destino_id: Option<EntidadId>,
    pub fecha: Option<NaiveDate>,
    pub folio: Option<String>,
    pub descripcion: Option<String>,
    pub renglones: Vec<MovimientoRenglon>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentoInventario {
    pub id: EntidadId,
    pub folio: String,
}
