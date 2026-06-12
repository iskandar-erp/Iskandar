//! Clientes (terceros del lado de la venta).

use serde::{Deserialize, Serialize};

use super::common::{EntidadId, Extra, IdFiscal, Moneda};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cliente {
    pub id: EntidadId,
    pub nombre: String,
    pub id_fiscal: Option<IdFiscal>,
    pub email: Option<String>,
    pub telefono: Option<String>,
    pub moneda: Option<Moneda>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuevoCliente {
    pub nombre: String,
    pub id_fiscal: Option<IdFiscal>,
    pub email: Option<String>,
    pub telefono: Option<String>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FiltroClientes {
    /// Búsqueda libre sobre nombre / identificador fiscal.
    pub texto: Option<String>,
    pub limite: Option<u32>,
    pub pagina: Option<u32>,
}
