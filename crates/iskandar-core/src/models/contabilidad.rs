//! Contabilidad: pólizas.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::common::{EntidadId, Extra};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Poliza {
    pub id: EntidadId,
    pub fecha: NaiveDate,
    pub concepto: Option<String>,
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FiltroPolizas {
    pub desde: Option<NaiveDate>,
    pub hasta: Option<NaiveDate>,
    pub limite: Option<u32>,
}
