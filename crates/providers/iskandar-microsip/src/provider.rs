//! Implementación del trait `ERPProvider` para Microsip.
//!
//! La DLL es síncrona y no thread-safe, así que cada operación corre en
//! `spawn_blocking` y `MicrosipDll` serializa el acceso con su `Mutex`.

use std::sync::Arc;

use async_trait::async_trait;
use iskandar_core::models::*;
use iskandar_core::{
    ClientesModule, ERPProvider, FacturasModule, InventarioModule, IskandarError, ProviderConfig,
    Result,
};

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::articulos::ArticulosMicrosip;
use crate::clientes::ClientesMicrosip;
use crate::dll::{MicrosipDll, RowReader};
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
}

impl MicrosipProvider {
    pub fn new(config: MicrosipConfig) -> Result<Self> {
        let dll = Arc::new(MicrosipDll::load(&config.dll_path)?);
        let clientes = ClientesMicrosip { dll: dll.clone(), config: config.clone() };
        let facturas = FacturasMicrosip { dll: dll.clone(), config: config.clone() };
        let articulos = ArticulosMicrosip { dll: dll.clone(), config: config.clone() };
        Ok(Self { config, dll, clientes, facturas, articulos })
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
}

struct FacturasMicrosip {
    dll: Arc<MicrosipDll>,
    config: MicrosipConfig,
}

#[async_trait]
impl FacturasModule for FacturasMicrosip {
    async fn crear(&self, factura: NuevaFactura) -> Result<Factura> {
        let dll = self.dll.clone();
        let config = self.config.clone();
        run_blocking(move || -> Result<Factura> {
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            // TODO: flujo completo según la referencia de Servicios Ventas:
            //   SetDBVentas(handle) → ChecaCompatibilidadVentas →
            //   SetReglasVentas → NuevaFactura → RenglonFactura por cada
            //   renglón → AplicaFactura → GetDoctoVeId para el id real.
            let _ = &factura;
            dll.disconnect(handle)?;
            Err(IskandarError::NotImplemented(
                "microsip::facturas::crear — flujo NuevaFactura/RenglonFactura/AplicaFactura pendiente",
            ))
        })
        .await
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
