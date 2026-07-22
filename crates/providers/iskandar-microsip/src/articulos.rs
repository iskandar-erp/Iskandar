//! Implementación de `InventarioModule` para Microsip.
//!
//! Lee ARTICULOS (PK: ARTICULO_ID INTEGER) con LEFT JOIN a PRECIOS_ARTICULOS
//! (filtrado a la lista "Precio de lista" vía PRECIOS_EMPRESA.ID_INTERNO='L')
//! para obtener `precio_lista`. Verificado en vivo contra 172.20.10.185
//! (ver Contexto/MEMORIA_AGENTES.md):
//! - `PRECIOS_EMPRESA` tiene 2 filas reales: PRECIO_EMPRESA_ID=42
//!   NOMBRE="Precio de lista" ID_INTERNO='L', y 43 "Precio mínimo" ID_INTERNO='M'.
//!   `ID_INTERNO='L'` es la convención estándar de Microsip para el precio
//!   de lista general y es estable entre instalaciones (a diferencia del id
//!   numérico 42, que es local a esta base) — mismo patrón que
//!   `ES_DIR_PPAL='S'` en clientes.rs: el filtro va en la cláusula ON.
//! - `PRECIOS_ARTICULOS` estaba vacía (0 filas) al momento de escribir esto,
//!   así que la escala del campo `PRECIO` (BIGINT) no pudo verificarse
//!   empíricamente contra una fila real. Ver `ESCALA_PRECIOS` abajo.
//!
//! `clave`: confirmado que ARTICULOS no tiene ninguna columna de clave
//! (41 columnas revisadas) — fuera de alcance.
//!
//! `existencia`: PENDIENTE, fuera de alcance de este cambio. EXIS_DISCRETOS
//! solo aplica a artículos "discretos" (con seguimiento por serie/lote vía
//! ARTICULOS_DISCRETOS, también vacía) — no es el mecanismo correcto para
//! artículos normales (ES_ALMACENABLE). La existencia general vive en
//! SALDOS_IN (ledger mensual ARTICULO_ID+ALMACEN_ID+ANO+MES con
//! ENTRADAS_UNIDADES/SALIDAS_UNIDADES), que requiere agregar sobre el
//! histórico para derivar un "stock actual" — diseño propio, con su propia
//! pregunta de escala de unidades, que merece su propio ciclo de
//! exploración de esquema antes de implementarse. Ver MEMORIA_AGENTES.md.

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

/// Escala del campo `PRECIO` de PRECIOS_ARTICULOS (BIGINT sin punto decimal).
///
/// SUPUESTO NO VERIFICADO: PRECIOS_ARTICULOS estaba vacía en 172.20.10.185
/// al implementar este mapeo, así que no hay fila real contra la que
/// comparar. escala=2 por analogía con los campos monetarios BIGINT de
/// DOCTOS_VE (ESCALA_MONTOS en provider.rs), donde SÍ está confirmado en
/// vivo (150000 = $1500.00, IVA 16% exacto). Es una constante separada
/// a propósito — precios y montos de factura son dominios distintos que
/// podrían divergir. VALIDAR contra una fila cruda de PRECIOS_ARTICULOS
/// en cuanto existan datos (deuda registrada en MEMORIA_AGENTES.md).
const ESCALA_PRECIOS: u32 = 2;

fn map_row(row: &RowReader) -> Result<Articulo> {
    let articulo_id = row.int_field("ARTICULO_ID")?;
    Ok(Articulo {
        id: EntidadId::Numerico(articulo_id as i64),
        nombre: row.str_field("NOMBRE")?,
        clave: None,
        precio_lista: row.opt_dec("PRECIO", ESCALA_PRECIOS),
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
        //
        // LEFT JOIN a PRECIOS_ARTICULOS filtrado (en el ON, no en el WHERE,
        // mismo patrón que ES_DIR_PPAL en clientes.rs) a la lista de precios
        // cuya PRECIOS_EMPRESA.ID_INTERNO = 'L' ("Precio de lista"). Un
        // artículo sin fila en esa lista queda con pa.PRECIO = NULL →
        // precio_lista = None (nunca 0).
        const JOIN_PRECIO: &str = "LEFT JOIN PRECIOS_ARTICULOS pa \
                 ON pa.ARTICULO_ID = a.ARTICULO_ID \
                 AND pa.PRECIO_EMPRESA_ID = ( \
                     SELECT pe.PRECIO_EMPRESA_ID FROM PRECIOS_EMPRESA pe \
                     WHERE pe.ID_INTERNO = 'L')";
        let (sql, owned_params) = match &filtro.texto {
            None => (
                format!(
                    "SELECT FIRST {limite} a.ARTICULO_ID, a.NOMBRE, pa.PRECIO \
                     FROM ARTICULOS a \
                     {JOIN_PRECIO} \
                     WHERE a.ESTATUS = 'A' \
                     ORDER BY a.NOMBRE"
                ),
                vec![],
            ),
            Some(texto) => (
                format!(
                    "SELECT FIRST {limite} a.ARTICULO_ID, a.NOMBRE, pa.PRECIO \
                     FROM ARTICULOS a \
                     {JOIN_PRECIO} \
                     WHERE a.ESTATUS = 'A' AND UPPER(a.NOMBRE) CONTAINING UPPER(:texto) \
                     ORDER BY a.NOMBRE"
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
