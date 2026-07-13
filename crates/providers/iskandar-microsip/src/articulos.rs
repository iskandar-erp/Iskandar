//! Implementación de `InventarioModule` para Microsip.
//!
//! Lee ARTICULOS (PK: ARTICULO_ID INTEGER). A diferencia de CLIENTES, esta
//! versión de la tabla no tiene columna CLAVE ni datos de precio/existencia
//! — esos viven en PRECIOS_ARTICULOS / EXISTENCIAS, todavía sin explorar
//! (ver Contexto/MEMORIA_AGENTES.md). `precio_lista` y `existencia` quedan
//! en `None` hasta implementar esos joins.

use std::sync::Arc;

use async_trait::async_trait;
use iskandar_core::{
    models::{Articulo, EntidadId, FiltroArticulos, NuevaEntrada, NuevaSalida, DocumentoInventario},
    IskandarError, InventarioModule, Result,
};

use crate::dll::{MicrosipDll, RowReader};
use crate::models::MicrosipConfig;
use crate::provider::run_blocking;

pub struct ArticulosMicrosip {
    pub(crate) dll: Arc<MicrosipDll>,
    pub(crate) config: MicrosipConfig,
}

fn map_row(row: &RowReader) -> Result<Articulo> {
    let articulo_id = row.int_field("ARTICULO_ID")?;
    Ok(Articulo {
        id: EntidadId::Numerico(articulo_id as i64),
        nombre: row.str_field("NOMBRE")?,
        clave: None,
        precio_lista: None,
        existencia: None,
        extra: Default::default(),
    })
}

#[async_trait]
impl InventarioModule for ArticulosMicrosip {
    async fn articulos(&self, filtro: FiltroArticulos) -> Result<Vec<Articulo>> {
        let dll = self.dll.clone();
        let config = self.config.clone();

        let limite = filtro.limite.unwrap_or(500);
        // `limite` es u32 de Rust — no es entrada del usuario, seguro interpolarlo.
        let (sql, owned_params) = match &filtro.texto {
            None => (
                format!(
                    "SELECT FIRST {limite} ARTICULO_ID, NOMBRE \
                     FROM ARTICULOS \
                     WHERE ESTATUS = 'A' \
                     ORDER BY NOMBRE"
                ),
                vec![],
            ),
            Some(texto) => (
                format!(
                    "SELECT FIRST {limite} ARTICULO_ID, NOMBRE \
                     FROM ARTICULOS \
                     WHERE ESTATUS = 'A' AND UPPER(NOMBRE) CONTAINING UPPER(:texto) \
                     ORDER BY NOMBRE"
                ),
                // El valor se pasa como parámetro enlazado — nunca interpolado.
                vec![("texto".to_string(), texto.clone())],
            ),
        };

        run_blocking(move || {
            let params: Vec<(&str, &str)> =
                owned_params.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            let resultado = dll.query(handle, &sql, &params, map_row);
            dll.disconnect(handle).ok();
            resultado
        })
        .await
    }

    async fn entrada(&self, _entrada: NuevaEntrada) -> Result<DocumentoInventario> {
        Err(IskandarError::NotImplemented(
            "microsip::inventario::entrada — flujo NuevaEntrada/RenglonEntrada/AplicaEntrada pendiente",
        ))
    }

    async fn salida(&self, _salida: NuevaSalida) -> Result<DocumentoInventario> {
        Err(IskandarError::NotImplemented(
            "microsip::inventario::salida — flujo NuevaSalida/RenglonSalida/AplicaSalida pendiente",
        ))
    }
}
