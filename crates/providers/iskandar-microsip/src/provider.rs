//! Implementación del trait `ERPProvider` para Microsip.
//!
//! La DLL es síncrona y no thread-safe, así que cada operación corre en
//! `spawn_blocking` y `MicrosipDll` serializa el acceso con su `Mutex`.

use std::sync::Arc;

use async_trait::async_trait;
use iskandar_core::models::*;
use iskandar_core::{
    ClientesModule, CxcModule, ERPProvider, FacturasModule, InventarioModule, IskandarError,
    ProviderConfig, Result,
};

use chrono::{Local, NaiveDate};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use crate::articulos::ArticulosMicrosip;
use crate::clientes::ClientesMicrosip;
use crate::cxc::CxcMicrosip;
use crate::dll::{FacturaParams, MicrosipDll, RenglonParams, RowReader};
use crate::models::MicrosipConfig;

const PAISES: &[Pais] = &[
    Pais::Mexico,
    Pais::Guatemala,
    Pais::ElSalvador,
    Pais::Honduras,
    Pais::Nicaragua,
    Pais::CostaRica,
    Pais::Panama,
];

pub struct MicrosipProvider {
    config: MicrosipConfig,
    dll: Arc<MicrosipDll>,
    clientes: ClientesMicrosip,
    facturas: FacturasMicrosip,
    articulos: ArticulosMicrosip,
    cxc: CxcMicrosip,
}

impl MicrosipProvider {
    pub fn new(config: MicrosipConfig) -> Result<Self> {
        let dll = Arc::new(MicrosipDll::load(&config.dll_path)?);
        let clientes = ClientesMicrosip { dll: dll.clone(), config: config.clone() };
        let facturas = FacturasMicrosip { dll: dll.clone(), config: config.clone() };
        let articulos = ArticulosMicrosip { dll: dll.clone(), config: config.clone() };
        let cxc = CxcMicrosip { dll: dll.clone(), config: config.clone() };
        Ok(Self { config, dll, clientes, facturas, articulos, cxc })
    }

    /// Fábrica para registrarse en el `ProviderRegistry`.
    pub fn from_provider_config(config: &ProviderConfig) -> Result<Arc<dyn ERPProvider>> {
        Ok(Arc::new(Self::new(config.typed::<MicrosipConfig>()?)?))
    }
}

#[async_trait]
impl ERPProvider for MicrosipProvider {
    fn name(&self) -> &'static str {
        "microsip"
    }

    fn version(&self) -> String {
        // TODO: consultar GetVersionApiVentasAsString una vez conectados.
        env!("CARGO_PKG_VERSION").to_string()
    }

    fn paises(&self) -> &[Pais] {
        PAISES
    }

    async fn probar_conexion(&self) -> Result<()> {
        let dll = self.dll.clone();
        let config = self.config.clone();
        run_blocking(move || {
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            if !dll.connected(handle)? {
                dll.disconnect(handle).ok();
                return Err(IskandarError::Connection(
                    "DBConnect retornó éxito pero DBConnected reporta desconectado".into(),
                ));
            }
            dll.disconnect(handle)
        })
        .await
    }

    fn clientes(&self) -> Option<&dyn ClientesModule> {
        Some(&self.clientes)
    }

    fn facturas(&self) -> Option<&dyn FacturasModule> {
        Some(&self.facturas)
    }

    fn inventario(&self) -> Option<&dyn InventarioModule> {
        Some(&self.articulos)
    }

    fn cxc(&self) -> Option<&dyn CxcModule> {
        Some(&self.cxc)
    }
}

struct FacturasMicrosip {
    dll: Arc<MicrosipDll>,
    config: MicrosipConfig,
}

#[async_trait]
impl FacturasModule for FacturasMicrosip {
    async fn crear(&self, factura: NuevaFactura) -> Result<Factura> {
        // El mapeo modelo-universal → parámetros DLL es trabajo síncrono
        // puro (sin tocar la dll), lo hacemos antes de spawn_blocking para
        // poder devolver errores de validación sin haber tocado la dll.
        let params = mapear_nueva_factura(&factura)?;

        let dll = self.dll.clone();
        let config = self.config.clone();

        // SetReglasVentas(ExistenciasNegativas, PrecioMinimo).
        //
        // OJO — inversión de sentido en `existencias_negativas`:
        // `MicrosipConfig::existencias_negativas` (models.rs) se documentó
        // como "permitir existencias negativas", pero el parámetro nativo
        // `ExistenciasNegativas` de `SetReglasVentas` significa lo
        // contrario: 1 = SÍ aplicar la regla que las prohíbe según las
        // preferencias de la empresa; 0 = ignorar la regla (o sea,
        // "permitir"). Por eso el bool se invierte aquí. `validar_precio_minimo`
        // SÍ coincide 1:1 con el parámetro `PrecioMinimo` (1 = validar).
        let reglas_ventas = (
            if config.existencias_negativas { 0 } else { 1 },
            if config.validar_precio_minimo { 1 } else { 0 },
        );

        let docto_id: i32 = run_blocking(move || {
            dll.crear_factura(
                &config.db_path,
                &config.usuario,
                &config.password,
                config.metadatos_path.as_deref(),
                reglas_ventas,
                params,
            )
        })
        .await?;

        // Reusa el mapeo ya validado en vivo de obtener() para construir
        // el `Factura` de retorno a partir del DOCTO_VE_ID real —
        // GetDoctoVeId ya se llamó dentro de crear_factura ANTES de
        // AplicaFactura (es el único momento válido, ver dll.rs), así que
        // aquí solo hace falta una lectura normal de solo lectura.
        self.obtener(&EntidadId::Numerico(docto_id as i64)).await
    }

    async fn obtener(&self, id: &EntidadId) -> Result<Factura> {
        let dll = self.dll.clone();
        let config = self.config.clone();

        // DOCTO_VE_ID es INTEGER en Firebird. Aceptamos Numerico o Texto parseable.
        let id_num: i64 = match id {
            EntidadId::Numerico(n) => *n,
            EntidadId::Texto(s) => s.parse::<i64>().map_err(|_| {
                IskandarError::Validation(format!(
                    "el id de factura debe ser numérico, recibido: '{s}'"
                ))
            })?,
        };

        run_blocking(move || {
            // id_num es i64 — solo dígitos — seguro interpolarlo en SQL.
            // TIPO_DOCTO = 'F' (Factura) confirmado en vivo con
            // `iskandar schema --tabla DOCTOS_VE --valores TIPO_DOCTO`
            // (valores reales: C, F, P). No filtramos por SUBTIPO_DOCTO
            // porque en esta base solo existe el valor 'N'.
            let sql = format!(
                "SELECT DOCTO_VE_ID, FOLIO, FECHA, CLIENTE_ID, IMPORTE_NETO, \
                        TOTAL_IMPUESTOS, FLETES, OTROS_CARGOS, DSCTO_IMPORTE, \
                        TOTAL_RETENCIONES \
                 FROM DOCTOS_VE \
                 WHERE TIPO_DOCTO = 'F' AND DOCTO_VE_ID = {id_num}"
            );
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            let resultado = dll.query(handle, &sql, &[], map_factura_row);
            dll.disconnect(handle).ok();
            resultado?
                .into_iter()
                .next()
                .ok_or_else(|| IskandarError::NotFound(format!("factura #{id_num}")))
        })
        .await
    }
}

/// Escala de los campos monetarios de DOCTOS_VE (NUMERIC almacenado como
/// BIGINT). Verificado en vivo con `schema --muestra`: una factura con
/// IMPORTE_NETO "150000" y TOTAL_IMPUESTOS "24000" — 24000/150000 = 16 %
/// exacto, el IVA mexicano — solo cuadra con escala 2 (1500.00 / 240.00).
const ESCALA_MONTOS: u32 = 2;

fn map_factura_row(row: &RowReader) -> Result<Factura> {
    let docto_id = row.int_field("DOCTO_VE_ID")?;
    let cliente_id = row.int_field("CLIENTE_ID")?;

    let importe_neto = monto_field(row, "IMPORTE_NETO")?;
    let total_impuestos = monto_field(row, "TOTAL_IMPUESTOS")?;
    let fletes = monto_field(row, "FLETES")?;
    let otros_cargos = monto_field(row, "OTROS_CARGOS")?;
    let dscto = monto_field(row, "DSCTO_IMPORTE")?;
    let retenciones = monto_field(row, "TOTAL_RETENCIONES")?;
    let total = importe_neto + total_impuestos + fletes + otros_cargos - dscto - retenciones;

    Ok(Factura {
        id: EntidadId::Numerico(docto_id as i64),
        folio: row.str_field("FOLIO")?,
        fecha: fecha_field(row, "FECHA")?,
        cliente_id: EntidadId::Numerico(cliente_id as i64),
        total: Some(total),
        extra: Default::default(),
    })
}

/// Lee un campo monetario (almacenado sin punto decimal, p. ej. "150000"
/// para $1,500.00) y aplica `ESCALA_MONTOS`.
fn monto_field(row: &RowReader, campo: &str) -> Result<Decimal> {
    let raw = row.str_field(campo)?;
    let entero: i64 = raw.trim().parse().map_err(|_| {
        IskandarError::Validation(format!("valor monetario no numérico en {campo}: {raw:?}"))
    })?;
    Ok(Decimal::new(entero, ESCALA_MONTOS))
}

/// Lee un campo de fecha en el formato "DD/MM/AAAA" que devuelve la DLL
/// (verificado en vivo con `schema --muestra` sobre DOCTOS_VE.FECHA).
fn fecha_field(row: &RowReader, campo: &str) -> Result<NaiveDate> {
    let raw = row.str_field(campo)?;
    NaiveDate::parse_from_str(raw.trim(), "%d/%m/%Y").map_err(|_| {
        IskandarError::Validation(format!(
            "formato de fecha inesperado en {campo}: {raw:?} (se esperaba DD/MM/AAAA)"
        ))
    })
}

// ============================================================================
// Mapeo NuevaFactura (modelo universal) → FacturaParams (dll.rs)
//
// Tabla de decisiones completa (con cita de la doc o "SUPUESTO NO
// VERIFICADO") en Contexto/MEMORIA_AGENTES.md. Resumen de los defaults
// que no vienen de `extra` ni del modelo universal:
// ============================================================================

/// 0 = dirección principal del cliente. Documentado (Refer.md L722-724).
const DEFAULT_DIR_CONSIG_ID: i64 = 0;
/// 0 = almacén principal de la empresa. Documentado (Refer.md L725-727).
const DEFAULT_ALMACEN_ID: i64 = 0;
/// SUPUESTO NO VERIFICADO: la doc no da default para TipoDscto/Descuento;
/// 'P' + 0.0 es un no-op inequívoco (0% de descuento extra).
const DEFAULT_TIPO_DSCTO: &str = "P";
const DEFAULT_DESCUENTO: f64 = 0.0;
/// Documentado como opcional (Refer.md L740).
const DEFAULT_ORDEN_COMPRA: &str = "";
/// SUPUESTO NO VERIFICADO: la doc no da default; 0.0 es el único valor
/// que no dispara los errores 13/15 ("artículo de fletes/otros cargos no
/// está definido en las preferencias de la empresa").
const DEFAULT_FLETES: f64 = 0.0;
const DEFAULT_OTROS_CARGOS: f64 = 0.0;
/// -1 = según las políticas de comisión de vendedores. Documentado
/// (Refer.md L747-748).
const DEFAULT_PCTJE_COMIS: f64 = -1.0;
/// 0 = condición de pago asociada al cliente. Documentado (Refer.md L761).
const DEFAULT_COND_PAGO_ID: i64 = 0;
/// 0 = vendedor asociado al cliente. Documentado (Refer.md L744).
const DEFAULT_VENDEDOR_ID: i64 = 0;
/// 0 = sin sustitución de impuesto. Documentado (Refer.md L768). Ambos
/// deben ir en 0, o ambos > 0 — nunca uno solo.
const DEFAULT_IMPTO_SUSTITUIDO_ID: i64 = 0;
const DEFAULT_IMPTO_SUSTITUTO_ID: i64 = 0;
/// Opcional; solo tiene efecto si se registra un importe de cobro
/// (Refer.md L770-771).
const DEFAULT_DESCRIPCION_COBRO: &str = "";
/// -1 = precio/descuento según políticas Microsip — política documentada
/// también a nivel de renglón (Refer.md L989-995) y ya reflejada en el
/// comentario de `Renglon::precio_unitario`/`descuento_pctje` en el core.
const DEFAULT_PRECIO_UNITARIO: f64 = -1.0;
const DEFAULT_PCTJE_DSCTO_RENGLON: f64 = -1.0;

/// Claves de `extra` que, si están presentes, forman el objeto
/// `PLHandle` de `NuevaFactura` (Refer.md, sección "Parámetros de la
/// lista"). Si ninguna está presente se pasa `PLHandle = -1` ("no hay
/// lista de parámetros, se toma el default de cada uno" — Refer.md
/// L780-781, confirmado explícitamente para NuevaFactura, no una
/// extrapolación de NuevoPedido).
const CLAVES_LISTA_PARAMETROS: &[&str] = &[
    "USO_CFDI",
    "LUGAR_EXPEDICION_ID",
    "FORMA_COBRO_ID",
    "CENTRO_COSTO_ID",
    "IMPUESTO_INCLUIDO",
];

fn mapear_nueva_factura(factura: &NuevaFactura) -> Result<FacturaParams> {
    let cliente_id = entidad_id_a_i32("cliente_id", &factura.cliente_id)?;

    // Fecha: la doc NO documenta ningún sentinela (p. ej. string vacío)
    // para "fecha del día" en NuevaFactura — a diferencia de otros
    // parámetros que sí llevan la palabra "opcional" explícita. Para no
    // depender de un comportamiento no documentado, cuando `fecha` es
    // `None` calculamos la fecha local de hoy y la formateamos
    // explícitamente, en vez de mandar "".
    let fecha_str = factura
        .fecha
        .unwrap_or_else(|| Local::now().date_naive())
        .format("%d/%m/%Y")
        .to_string();

    // Folio: confirmado en la sección NuevoPedido (mismo formato PAnsiChar,
    // mismos valores permitidos que NuevaFactura, Refer.md L702-719):
    // "" (string vacío) = "se asigna el siguiente folio 'sin serie'".
    let folio_str = factura.folio.clone().unwrap_or_default();

    // `moneda`: NuevaFactura (a diferencia de NuevoPedido, que sí trae
    // MonedaId) NO tiene ningún parámetro de moneda en su firma nativa
    // (ApiMspVentasExt.pas L32-38 vs L20-23) — no hay forma de forzar la
    // moneda de una factura por esta Api. Si el llamador pide una moneda
    // explícita, lo ignoramos silenciosamente aquí (Microsip usará la
    // moneda del cliente); queda documentado como limitación conocida.
    let _ = &factura.moneda;

    let descripcion = factura.descripcion.clone().unwrap_or_default();

    let extra = &factura.extra;

    let dir_consig_id = extra_i64(extra, "DirConsigId", DEFAULT_DIR_CONSIG_ID)? as i32;
    let almacen_id = extra_i64(extra, "AlmacenId", DEFAULT_ALMACEN_ID)? as i32;
    let tipo_dscto = extra_string(extra, "TipoDscto", DEFAULT_TIPO_DSCTO)?;
    let descuento = extra_f64(extra, "Descuento", DEFAULT_DESCUENTO)?;
    let orden_compra = extra_string(extra, "OrdenCompra", DEFAULT_ORDEN_COMPRA)?;
    let fletes = extra_f64(extra, "Fletes", DEFAULT_FLETES)?;
    let otros_cargos = extra_f64(extra, "OtrosCargos", DEFAULT_OTROS_CARGOS)?;
    let pctje_comis = extra_f64(extra, "PctjeComis", DEFAULT_PCTJE_COMIS)?;
    let cond_pago_id = extra_i64(extra, "CondPagoId", DEFAULT_COND_PAGO_ID)? as i32;
    let vendedor_id = extra_i64(extra, "VendedorId", DEFAULT_VENDEDOR_ID)? as i32;
    let impto_sustituido_id =
        extra_i64(extra, "ImptoSustituidoId", DEFAULT_IMPTO_SUSTITUIDO_ID)? as i32;
    let impto_sustituto_id =
        extra_i64(extra, "ImptoSustitutoId", DEFAULT_IMPTO_SUSTITUTO_ID)? as i32;
    let descripcion_cobro = extra_string(extra, "DescripcionCobro", DEFAULT_DESCRIPCION_COBRO)?;

    // ImporteCobro: DELIBERADAMENTE SIN DEFAULT. -1 registra el cobro
    // TOTAL de la factura automáticamente (Refer.md L774); no existe
    // ningún sentinela documentado para "no registrar cobro" en una
    // factura a crédito válida (no PPD). Defaultear a -1 marcaría
    // silenciosamente cualquier factura a crédito como pagada en su
    // totalidad — corrupción de cuentas por cobrar reales. Decisión
    // confirmada con el supervisor: exigir que el llamador lo especifique
    // explícitamente, sin inventar 0 ni -1.
    let importe_cobro = importe_cobro_requerido(extra)?;

    let lista_parametros = construir_lista_parametros(extra);

    let dir_cliente_id = extra
        .get("DirClienteId")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32);

    let formas_cobro_cbb: Vec<i32> = extra
        .get("FormasCobroCbb")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).map(|n| n as i32).collect())
        .unwrap_or_default();

    let renglones = factura
        .renglones
        .iter()
        .map(mapear_renglon)
        .collect::<Result<Vec<_>>>()?;
    if renglones.is_empty() {
        return Err(IskandarError::Validation(
            "una factura necesita al menos un renglón (AplicaFactura retorna error \
             2 = 'No se han registrado renglones en la factura' si se manda vacía)"
                .into(),
        ));
    }

    Ok(FacturaParams {
        fecha: fecha_str,
        folio: folio_str,
        cliente_id,
        dir_consig_id,
        almacen_id,
        tipo_dscto,
        descuento,
        orden_compra,
        descripcion,
        fletes,
        otros_cargos,
        pctje_comis,
        cond_pago_id,
        vendedor_id,
        impto_sustituido_id,
        impto_sustituto_id,
        importe_cobro,
        descripcion_cobro,
        lista_parametros,
        dir_cliente_id,
        formas_cobro_cbb,
        renglones,
    })
}

fn mapear_renglon(renglon: &Renglon) -> Result<RenglonParams> {
    let articulo_id = entidad_id_a_i32("articulo_id", &renglon.articulo_id)?;
    let unidades = dec_to_f64("unidades", renglon.unidades)?;
    let precio_unitario = match renglon.precio_unitario {
        Some(p) => dec_to_f64("precio_unitario", p)?,
        None => DEFAULT_PRECIO_UNITARIO,
    };
    let pctje_dscto = match renglon.descuento_pctje {
        Some(p) => dec_to_f64("descuento_pctje", p)?,
        None => DEFAULT_PCTJE_DSCTO_RENGLON,
    };
    Ok(RenglonParams {
        articulo_id,
        unidades,
        precio_unitario,
        pctje_dscto,
        notas: renglon.notas.clone().unwrap_or_default(),
    })
}

pub(crate) fn entidad_id_a_i32(campo: &str, id: &EntidadId) -> Result<i32> {
    match id {
        EntidadId::Numerico(n) => Ok(*n as i32),
        EntidadId::Texto(s) => Err(IskandarError::Validation(format!(
            "{campo} debe ser numérico en Microsip, recibido: '{s}'"
        ))),
    }
}

pub(crate) fn dec_to_f64(campo: &str, valor: Decimal) -> Result<f64> {
    valor
        .to_f64()
        .ok_or_else(|| IskandarError::Validation(format!("no se pudo convertir {campo}={valor} a f64")))
}

pub(crate) fn extra_i64(extra: &Extra, clave: &str, default: i64) -> Result<i64> {
    match extra.get(clave) {
        None => Ok(default),
        Some(v) => v
            .as_i64()
            .ok_or_else(|| IskandarError::Validation(format!("extra.{clave} debe ser entero"))),
    }
}

pub(crate) fn extra_f64(extra: &Extra, clave: &str, default: f64) -> Result<f64> {
    match extra.get(clave) {
        None => Ok(default),
        Some(v) => v
            .as_f64()
            .ok_or_else(|| IskandarError::Validation(format!("extra.{clave} debe ser numérico"))),
    }
}

pub(crate) fn extra_string(extra: &Extra, clave: &str, default: &str) -> Result<String> {
    match extra.get(clave) {
        None => Ok(default.to_string()),
        Some(v) => v
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| IskandarError::Validation(format!("extra.{clave} debe ser texto"))),
    }
}

/// ImporteCobro es el único parámetro de `extra` sin default seguro —
/// ver el comentario extenso en `mapear_nueva_factura`. Error de
/// validación explícito si falta, nunca un 0/-1 adivinado.
fn importe_cobro_requerido(extra: &Extra) -> Result<f64> {
    match extra.get("ImporteCobro") {
        Some(v) => v.as_f64().ok_or_else(|| {
            IskandarError::Validation("extra.ImporteCobro debe ser numérico".into())
        }),
        None => Err(IskandarError::Validation(
            "extra.ImporteCobro es obligatorio y no tiene default seguro: -1 registra \
             el cobro TOTAL de la factura automáticamente; no existe ningún sentinela \
             documentado para \"sin cobro\" en una factura a crédito válida. \
             Especifícalo explícitamente (-1 para contado total, o el importe exacto \
             a cobrar hoy; 0 es un SUPUESTO NO VERIFICADO — no lo asumas sin probarlo \
             contra una base de PRUEBAS)."
                .into(),
        )),
    }
}

/// Arma los pares (nombre, valor) del objeto `PLHandle` a partir de las
/// claves de `extra` que Microsip reconoce para `NuevaFactura`. Los
/// valores numéricos/booleanos se serializan a texto plano (sin comillas
/// JSON) porque `PLSetParamValue` espera `PAnsiChar` para TODOS los
/// valores, incluidos los que son IDs (ApiMspBasicaExt.pas L170).
fn construir_lista_parametros(extra: &Extra) -> Vec<(String, String)> {
    CLAVES_LISTA_PARAMETROS
        .iter()
        .filter_map(|clave| {
            extra.get(*clave).map(|v| (clave.to_string(), valor_json_a_texto(v)))
        })
        .collect()
}

pub(crate) fn valor_json_a_texto(v: &serde_json::Value) -> String {
    match v.as_str() {
        Some(s) => s.to_string(),
        // Números/bools: `to_string()` de serde_json::Value ya produce
        // la forma sin comillas para estos casos (p. ej. `42`, `true`).
        None => v.to_string(),
    }
}

/// Columna de una tabla Firebird, para el subcomando `iskandar schema`.
#[derive(Debug)]
pub struct CampoSchema {
    pub nombre: String,
    pub tipo: String,
}

impl MicrosipProvider {
    /// Lista todas las tablas de usuario de la base de empresa.
    pub async fn listar_tablas(&self) -> Result<Vec<String>> {
        let dll = self.dll.clone();
        let config = self.config.clone();
        run_blocking(move || {
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            let resultado = dll.query(
                handle,
                "SELECT TRIM(RDB$RELATION_NAME) AS TABLA \
                 FROM RDB$RELATIONS \
                 WHERE RDB$SYSTEM_FLAG = 0 AND RDB$VIEW_SOURCE IS NULL \
                 ORDER BY RDB$RELATION_NAME",
                &[],
                |row| row.str_field("TABLA"),
            );
            dll.disconnect(handle).ok();
            resultado
        })
        .await
    }

    /// Describe las columnas de una tabla (nombre y tipo Firebird).
    pub async fn describir_tabla(&self, tabla: &str) -> Result<Vec<CampoSchema>> {
        let dll = self.dll.clone();
        let config = self.config.clone();
        let tabla = tabla.to_uppercase();
        run_blocking(move || {
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            let resultado = dll.query(
                handle,
                "SELECT TRIM(f.RDB$FIELD_NAME) AS CAMPO, \
                        fs.RDB$FIELD_TYPE AS TIPO_ID, \
                        fs.RDB$FIELD_LENGTH AS LONGITUD \
                 FROM RDB$RELATION_FIELDS f \
                 JOIN RDB$FIELDS fs ON f.RDB$FIELD_SOURCE = fs.RDB$FIELD_NAME \
                 WHERE TRIM(f.RDB$RELATION_NAME) = :tabla \
                 ORDER BY f.RDB$FIELD_POSITION",
                &[("tabla", &tabla)],
                |row| {
                    let nombre = row.str_field("CAMPO")?;
                    let tipo_id = row.int_field("TIPO_ID")?;
                    let longitud = row.int_field("LONGITUD").unwrap_or(0);
                    Ok(CampoSchema {
                        nombre,
                        tipo: tipo_firebird(tipo_id, longitud),
                    })
                },
            );
            dll.disconnect(handle).ok();
            resultado
        })
        .await
    }

    /// Lista los valores distintos que existen para `campo` en `tabla`.
    /// Solo para diagnóstico manual (`iskandar schema --tabla T --valores C`);
    /// `tabla` y `campo` deben ser identificadores SQL válidos (no vienen
    /// de la API HTTP, solo del CLI local), así que se validan aquí en vez
    /// de enlazarse como parámetro — Firebird no permite bind params en
    /// nombres de columna/tabla.
    pub async fn valores_distintos(&self, tabla: &str, campo: &str) -> Result<Vec<String>> {
        for ident in [tabla, campo] {
            if ident.is_empty() || !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(IskandarError::Validation(format!(
                    "identificador SQL inválido: '{ident}'"
                )));
            }
        }
        let dll = self.dll.clone();
        let config = self.config.clone();
        let tabla = tabla.to_uppercase();
        let campo = campo.to_uppercase();
        run_blocking(move || {
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            let sql = format!("SELECT DISTINCT TRIM({campo}) AS VALOR FROM {tabla} ORDER BY VALOR");
            let resultado = dll.query(handle, &sql, &[], |row| row.str_field("VALOR"));
            dll.disconnect(handle).ok();
            resultado
        })
        .await
    }

    /// Imprime las primeras `limite` filas de `tabla`, todas las columnas
    /// como texto (vía `DtstGetFieldAsString`, que formatea cualquier tipo
    /// — fechas, decimales, etc. — como lo haría la UI de Microsip).
    /// Solo para diagnóstico manual antes de implementar el mapeo real de
    /// una entidad nueva.
    pub async fn muestra_tabla(&self, tabla: &str, limite: u32) -> Result<Vec<Vec<(String, String)>>> {
        let campos = self.describir_tabla(tabla).await?;
        let nombres: Vec<String> = campos.into_iter().map(|c| c.nombre).collect();
        if nombres.is_empty() {
            return Ok(vec![]);
        }

        let dll = self.dll.clone();
        let config = self.config.clone();
        let tabla = tabla.to_uppercase();
        let cols_sql = nombres.join(", ");
        run_blocking(move || {
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            let sql = format!("SELECT FIRST {limite} {cols_sql} FROM {tabla}");
            let resultado = dll.query(handle, &sql, &[], |row| {
                Ok(nombres
                    .iter()
                    .map(|n| {
                        let valor = row.str_field(n).unwrap_or_else(|e| format!("<error: {e}>"));
                        (n.clone(), valor)
                    })
                    .collect())
            });
            dll.disconnect(handle).ok();
            resultado
        })
        .await
    }
}

fn tipo_firebird(id: i32, longitud: i32) -> String {
    match id {
        14 => format!("CHAR({longitud})"),
        37 => format!("VARCHAR({longitud})"),
        7 => "SMALLINT".to_string(),
        8 => "INTEGER".to_string(),
        16 => "BIGINT".to_string(),
        10 => "FLOAT".to_string(),
        27 => "DOUBLE PRECISION".to_string(),
        12 => "DATE".to_string(),
        13 => "TIME".to_string(),
        35 => "TIMESTAMP".to_string(),
        261 => "BLOB".to_string(),
        _ => format!("TIPO_{id}"),
    }
}

/// Corre trabajo síncrono de la DLL fuera del executor async.
pub(crate) async fn run_blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| IskandarError::Connection(format!("tarea bloqueante falló: {e}")))?
}
