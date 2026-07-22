//! Implementación de `CxcModule` para Microsip.
//!
//! Crea créditos de Cuentas por Cobrar (cobros, notas de crédito, etc.)
//! vía el módulo "Servicios Cxc" de `ApiMicrosip.dll` (`ApiMspCxcExt.pas`).
//! El flujo FFI completo vive en `dll.rs::MicrosipDll::crear_credito`; este
//! archivo solo mapea el modelo universal `NuevoCredito` a los
//! parámetros nativos y construye el `Credito{id, folio}` de retorno.
//!
//! A diferencia de facturas (que reusan `FacturasMicrosip::obtener()`, ya
//! validado en vivo), el trait `CxcModule` no tiene ningún método de
//! lectura — se escribe aquí una query mínima nueva contra `DOCTOS_CC`,
//! con las columnas confirmadas en vivo con
//! `iskandar schema --provider microsip --tabla DOCTOS_CC`.
//!
//! ## Limitaciones conocidas de esta v1 (documentadas también en
//! `Contexto/MEMORIA_AGENTES.md`):
//! - Sin desglose de impuestos personalizado por cargo (`RenglonCreditoImpuestoCc`
//!   no se implementa): se usa siempre el desglose automático de Microsip.
//! - `TipoImporte` de `RenglonCreditoCc` fijo en `'R'` (importe explícito):
//!   los modos `'A'` (saldo por acreditar), `'L'` (liquidar por
//!   antigüedad) y `'S'` (liquidar saldo completo) quedan fuera de
//!   alcance, sin plumbing de override vía `extra`.
//! - Anticipos (`NuevoAnticipoCc`/`AplicaAnticipoCc`) completamente fuera
//!   de alcance: firma distinta, sin representación en el modelo universal.

use std::sync::Arc;

use async_trait::async_trait;
use iskandar_core::{
    models::{Credito, EntidadId, Extra, NuevoCredito},
    CxcModule, IskandarError, Result,
};

use chrono::Local;

use crate::dll::{AplicacionCreditoParams, CreditoParams, MicrosipDll, RowReader};
use crate::models::MicrosipConfig;
use crate::provider::{dec_to_f64, entidad_id_a_i32, extra_f64, extra_i64, run_blocking, valor_json_a_texto};

pub struct CxcMicrosip {
    pub(crate) dll: Arc<MicrosipDll>,
    pub(crate) config: MicrosipConfig,
}

// ============================================================================
// Defaults documentados de NuevoCreditoCc / RenglonCreditoCc
// (Api - Servicios Cxc - Refer.md). Mismo formato que provider.rs para
// NuevaFactura: cada default con cita textual o `SUPUESTO NO VERIFICADO`
// explícito.
// ============================================================================

/// 0 = cobrador asignado al cliente (Refer.md L312-313: "si el concepto
/// del crédito no integra comisiones, este dato no importa"). El modelo
/// universal `NuevoCredito` no tiene ningún campo de cobrador — solo
/// llega vía `extra.CobradorId`.
const DEFAULT_COBRADOR_ID: i64 = 0;
/// "" = folio automático (mismo sentinela documentado que en NuevaFactura).
const DEFAULT_FOLIO: &str = "";
/// "" = sin descripción (campo de texto libre, opcional).
const DEFAULT_DESCRIPCION: &str = "";
/// 0 = el cargo se identifica solo por `folio_cargo`. SUPUESTO NO
/// VERIFICADO (ver nota extensa en `dll::AplicacionCreditoParams::cargo_id`).
const DEFAULT_CARGO_ID: i64 = 0;
/// "" = sin folio de cargo explícito.
const DEFAULT_FOLIO_CARGO: &str = "";
/// -1.0 = "determinarlo automáticamente" (Refer.md L576). Override a 0.0
/// vía `extra.DsctoPpag` si la preferencia "Emitir CFDI de los cobros"
/// está activa (Refer.md L579-580) — aplica UNIFORME a todos los
/// renglones del crédito porque es una preferencia de empresa, no un
/// dato por-cargo (`AplicacionCargo` del modelo universal no trae este
/// campo).
const DEFAULT_DSCTO_PPAG: f64 = -1.0;

/// Claves de `extra` que, si están presentes, forman el objeto `PLHandle`
/// de `NuevoCreditoCc` (Refer.md, sección "Parámetros de la lista",
/// L320-452). Ninguna es obligatoria de forma incondicional para el caso
/// base (crédito normal sin forma de cobro específica) — todas las
/// obligatoriedades documentadas son condicionales a concepto tipo "Pago"
/// o forma de cobro bancarizada, fuera del caso mínimo v1 salvo que el
/// llamador las pase explícitamente vía `extra`.
const CLAVES_LISTA_PARAMETROS: &[&str] = &[
    "ES_COBRO_POR_DEPOSITAR",
    "FORMA_COBRO_ID",
    "REFERENCIA",
    "FECHA_APLICACION",
    "IMPORTE_COBRO",
    "CUENTA_BAN_ID",
    "BANCO_ORIGEN_ID",
    "NUM_CUENTA_ORIGEN",
    "ARCHIVO_CEP",
    "REFER_MOVTO_BA",
    "LUGAR_EXPEDICION_ID",
    "USO_CFDI",
    "SUCURSAL_ID",
];

#[async_trait]
impl CxcModule for CxcMicrosip {
    async fn crear_credito(&self, credito: NuevoCredito) -> Result<Credito> {
        // Mapeo modelo-universal → parámetros dll es trabajo síncrono
        // puro (sin tocar la dll), igual que `mapear_nueva_factura` en
        // provider.rs — se hace antes de `spawn_blocking` para devolver
        // errores de validación sin haber tocado la dll.
        let params = mapear_nuevo_credito(&credito)?;

        let dll = self.dll.clone();
        let config = self.config.clone();

        let docto_id: i32 = run_blocking(move || {
            dll.crear_credito(
                &config.db_path,
                &config.usuario,
                &config.password,
                config.metadatos_path.as_deref(),
                params,
            )
        })
        .await?;

        // No existe ningún método `obtener()` previo para créditos que
        // reusar (a diferencia de `FacturasMicrosip::crear`, que reusa
        // `obtener()`) — query mínima nueva contra DOCTOS_CC, confirmada
        // en vivo (`iskandar schema --tabla DOCTOS_CC`): DOCTO_CC_ID
        // (PK, INTEGER) y FOLIO (CHAR(9)) son suficientes para el
        // `Credito{id, folio}` MUY minimal del modelo universal — no
        // hace falta leer ni mapear renglones/importes de vuelta.
        let dll = self.dll.clone();
        let config = self.config.clone();
        run_blocking(move || {
            // docto_id es i32 devuelto por GetDoctoCCId — no es entrada
            // del usuario, seguro interpolarlo en SQL.
            let sql = format!(
                "SELECT DOCTO_CC_ID, FOLIO FROM DOCTOS_CC WHERE DOCTO_CC_ID = {docto_id}"
            );
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            let resultado = dll.query(handle, &sql, &[], map_credito_row);
            dll.disconnect(handle).ok();
            resultado?
                .into_iter()
                .next()
                .ok_or_else(|| IskandarError::NotFound(format!("crédito Cxc #{docto_id}")))
        })
        .await
    }
}

fn map_credito_row(row: &RowReader) -> Result<Credito> {
    let docto_id = row.int_field("DOCTO_CC_ID")?;
    Ok(Credito {
        id: EntidadId::Numerico(docto_id as i64),
        folio: row.str_field("FOLIO")?,
    })
}

fn mapear_nuevo_credito(credito: &NuevoCredito) -> Result<CreditoParams> {
    let cliente_id = entidad_id_a_i32("cliente_id", &credito.cliente_id)?;
    // concepto_id es obligatorio en el modelo universal (no Option) — el
    // llamador decide si pasar 0 explícitamente, que Microsip interpreta
    // como el concepto "Cobro" (Refer.md L295-296: "asignar el Id del
    // concepto Cobro").
    let concepto_cc_id = entidad_id_a_i32("concepto_id", &credito.concepto_id)?;

    // Fecha: NuevoCredito.fecha es NaiveDateTime (siempre trae hora,
    // aunque sea medianoche) — a diferencia de NuevaFactura.fecha
    // (NaiveDate). Formato "D/M/A HH:NN" con BARRA confirmado en la doc
    // (Refer.md L297-298: "la hora... es opcional en la captura", pero
    // incluirla siempre es seguro). Si `fecha` es `None`, usamos la
    // fecha/hora local de hoy, igual que `mapear_nueva_factura`.
    let fecha_str = credito
        .fecha
        .unwrap_or_else(|| Local::now().naive_local())
        .format("%d/%m/%Y %H:%M")
        .to_string();

    let folio_str = credito.folio.clone().unwrap_or_else(|| DEFAULT_FOLIO.to_string());
    let descripcion = credito.descripcion.clone().unwrap_or_else(|| DEFAULT_DESCRIPCION.to_string());

    let extra = &credito.extra;
    let cobrador_id = extra_i64(extra, "CobradorId", DEFAULT_COBRADOR_ID)? as i32;
    let dscto_ppag_default = extra_f64(extra, "DsctoPpag", DEFAULT_DSCTO_PPAG)?;

    let lista_parametros = construir_lista_parametros(extra);

    if credito.aplicaciones.is_empty() {
        return Err(IskandarError::Validation(
            "un crédito necesita al menos una aplicación contra un cargo (mapea a \
             RenglonCreditoCc; AplicaCreditoCc fallaría con un crédito sin renglones)"
                .into(),
        ));
    }

    let aplicaciones = credito
        .aplicaciones
        .iter()
        .map(|aplicacion| mapear_aplicacion(aplicacion, dscto_ppag_default))
        .collect::<Result<Vec<_>>>()?;

    Ok(CreditoParams {
        concepto_cc_id,
        fecha: fecha_str,
        folio: folio_str,
        cliente_id,
        descripcion,
        cobrador_id,
        lista_parametros,
        aplicaciones,
    })
}

fn mapear_aplicacion(
    aplicacion: &iskandar_core::models::AplicacionCargo,
    dscto_ppag: f64,
) -> Result<AplicacionCreditoParams> {
    let cargo_id = match &aplicacion.cargo_id {
        Some(id) => entidad_id_a_i32("aplicaciones[].cargo_id", id)?,
        None => DEFAULT_CARGO_ID as i32,
    };
    let folio_cargo = aplicacion
        .folio_cargo
        .clone()
        .unwrap_or_else(|| DEFAULT_FOLIO_CARGO.to_string());
    let importe = dec_to_f64("aplicaciones[].importe", aplicacion.importe)?;

    Ok(AplicacionCreditoParams {
        cargo_id,
        folio_cargo,
        importe,
        dscto_ppag,
    })
}

/// Arma los pares (nombre, valor) del objeto `PLHandle` a partir de las
/// claves de `extra` que Microsip reconoce para `NuevoCreditoCc` — mismo
/// patrón que `construir_lista_parametros` en provider.rs (Ventas), pero
/// con las 13 claves propias de Cxc (`CLAVES_LISTA_PARAMETROS` arriba).
fn construir_lista_parametros(extra: &Extra) -> Vec<(String, String)> {
    CLAVES_LISTA_PARAMETROS
        .iter()
        .filter_map(|clave| extra.get(*clave).map(|v| (clave.to_string(), valor_json_a_texto(v))))
        .collect()
}
