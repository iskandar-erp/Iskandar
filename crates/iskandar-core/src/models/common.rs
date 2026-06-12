//! Tipos transversales: identidad, país, moneda, renglones.

use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Identificador universal de entidades.
///
/// Los ERPs de escritorio (Microsip, Aspel, CONTPAQi) usan enteros de
/// Firebird/SQL; los ERPs cloud (Siigo, Alegra, Defontana) usan UUIDs o
/// strings. El core soporta ambos para no castigar a ningún provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EntidadId {
    Numerico(i64),
    Texto(String),
}

impl EntidadId {
    /// Interpreta un string (p. ej. un segmento de URL): si parsea como
    /// entero es `Numerico`, si no, `Texto`.
    pub fn parse(s: &str) -> Self {
        s.parse::<i64>()
            .map(EntidadId::Numerico)
            .unwrap_or_else(|_| EntidadId::Texto(s.to_string()))
    }

    pub fn como_i64(&self) -> Option<i64> {
        match self {
            EntidadId::Numerico(n) => Some(*n),
            EntidadId::Texto(_) => None,
        }
    }
}

impl From<i64> for EntidadId {
    fn from(n: i64) -> Self {
        EntidadId::Numerico(n)
    }
}

impl From<&str> for EntidadId {
    fn from(s: &str) -> Self {
        EntidadId::Texto(s.to_string())
    }
}

impl fmt::Display for EntidadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntidadId::Numerico(n) => write!(f, "{n}"),
            EntidadId::Texto(s) => write!(f, "{s}"),
        }
    }
}

/// Países del alcance continental de Iskandar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pais {
    Mexico,
    Guatemala,
    ElSalvador,
    Honduras,
    Nicaragua,
    CostaRica,
    Panama,
    RepublicaDominicana,
    Colombia,
    Venezuela,
    Ecuador,
    Peru,
    Bolivia,
    Chile,
    Argentina,
    Paraguay,
    Uruguay,
    Brasil,
}

/// Monedas ISO 4217 de la región (más USD, que circula en varias).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Moneda {
    MXN,
    GTQ,
    HNL,
    NIO,
    CRC,
    PAB,
    DOP,
    COP,
    VES,
    PEN,
    BOB,
    CLP,
    ARS,
    PYG,
    UYU,
    BRL,
    USD,
}

/// Identificador fiscal según país: RFC (México), NIT (Colombia,
/// Guatemala), RUC (Perú, Ecuador, Paraguay), RUT (Chile, Uruguay),
/// CUIT (Argentina), CNPJ (Brasil).
///
/// La validación de formato es responsabilidad de cada provider — cada
/// autoridad fiscal tiene sus propias reglas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdFiscal {
    pub pais: Pais,
    pub valor: String,
}

/// Renglón de documento (factura, pedido, orden de compra).
///
/// Dinero y cantidades en `Decimal` — nunca `f64`: estos números
/// terminan en comprobantes fiscales.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Renglon {
    pub articulo_id: EntidadId,
    pub unidades: Decimal,
    /// `None` deja que el ERP determine el precio según sus políticas
    /// (en Microsip equivale a pasar -1 a `RenglonFactura`).
    pub precio_unitario: Option<Decimal>,
    /// Porcentaje de descuento; `None` = el que dicte el ERP.
    pub descuento_pctje: Option<Decimal>,
    pub notas: Option<String>,
}

/// Campos específicos del ERP que no mapean al modelo universal.
pub type Extra = serde_json::Map<String, serde_json::Value>;
