//! Implementación de `ClientesModule` para Microsip.
//!
//! Lee CLIENTES (PK: CLIENTE_ID INTEGER) con LEFT JOIN a DIRS_CLIENTES
//! para obtener RFC_CURP, TELEFONO1 y EMAIL de la dirección principal
//! (ES_DIR_PPAL = 'S').

use std::sync::Arc;

use async_trait::async_trait;
use iskandar_core::{
    models::{Cliente, EntidadId, FiltroClientes, IdFiscal, NuevoCliente, Pais},
    ClientesModule, IskandarError, Result,
};

use crate::dll::{MicrosipDll, RowReader};
use crate::models::MicrosipConfig;
use crate::provider::run_blocking;

pub struct ClientesMicrosip {
    pub(crate) dll: Arc<MicrosipDll>,
    pub(crate) config: MicrosipConfig,
}

fn map_row(row: &RowReader) -> Result<Cliente> {
    let cliente_id = row.int_field("CLIENTE_ID")?;
    Ok(Cliente {
        id: EntidadId::Numerico(cliente_id as i64),
        nombre: row.str_field("NOMBRE")?,
        id_fiscal: row
            .opt_str("RFC_CURP")
            .map(|r| IdFiscal { pais: Pais::Mexico, valor: r }),
        telefono: row.opt_str("TELEFONO1"),
        email: row.opt_str("EMAIL"),
        moneda: None,
        extra: Default::default(),
    })
}

#[async_trait]
impl ClientesModule for ClientesMicrosip {
    async fn listar(&self, filtro: FiltroClientes) -> Result<Vec<Cliente>> {
        let dll = self.dll.clone();
        let config = self.config.clone();

        let limite = filtro.limite.unwrap_or(500);
        // `limite` es u32 de Rust — no es entrada del usuario, seguro interpolarlo.
        let (sql, owned_params) = match &filtro.texto {
            None => (
                format!(
                    "SELECT FIRST {limite} \
                         c.CLIENTE_ID, c.NOMBRE, d.RFC_CURP, d.TELEFONO1, d.EMAIL \
                     FROM CLIENTES c \
                     LEFT JOIN DIRS_CLIENTES d \
                         ON d.CLIENTE_ID = c.CLIENTE_ID AND d.ES_DIR_PPAL = 'S' \
                     ORDER BY c.NOMBRE"
                ),
                vec![],
            ),
            Some(texto) => (
                format!(
                    "SELECT FIRST {limite} \
                         c.CLIENTE_ID, c.NOMBRE, d.RFC_CURP, d.TELEFONO1, d.EMAIL \
                     FROM CLIENTES c \
                     LEFT JOIN DIRS_CLIENTES d \
                         ON d.CLIENTE_ID = c.CLIENTE_ID AND d.ES_DIR_PPAL = 'S' \
                     WHERE UPPER(c.NOMBRE) CONTAINING UPPER(:texto) \
                        OR d.RFC_CURP CONTAINING :texto \
                     ORDER BY c.NOMBRE"
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

    async fn obtener(&self, id: &EntidadId) -> Result<Cliente> {
        let dll = self.dll.clone();
        let config = self.config.clone();

        // CLIENTE_ID es INTEGER en Firebird. Aceptamos Numerico o Texto parseable.
        let id_num: i64 = match id {
            EntidadId::Numerico(n) => *n,
            EntidadId::Texto(s) => s.parse::<i64>().map_err(|_| {
                IskandarError::Validation(format!(
                    "el id de cliente debe ser numérico, recibido: '{s}'"
                ))
            })?,
        };

        run_blocking(move || {
            // id_num es i64 — solo contiene dígitos — seguro interpolarlo en SQL.
            let sql = format!(
                "SELECT c.CLIENTE_ID, c.NOMBRE, d.RFC_CURP, d.TELEFONO1, d.EMAIL \
                 FROM CLIENTES c \
                 LEFT JOIN DIRS_CLIENTES d \
                     ON d.CLIENTE_ID = c.CLIENTE_ID AND d.ES_DIR_PPAL = 'S' \
                 WHERE c.CLIENTE_ID = {id_num}"
            );
            let handle = dll.connect(&config.db_path, &config.usuario, &config.password)?;
            let resultado = dll.query(handle, &sql, &[], map_row);
            dll.disconnect(handle).ok();
            resultado?
                .into_iter()
                .next()
                .ok_or_else(|| IskandarError::NotFound(format!("cliente #{id_num}")))
        })
        .await
    }

    async fn crear(&self, _cliente: NuevoCliente) -> Result<Cliente> {
        Err(IskandarError::NotImplemented("microsip::clientes::crear"))
    }
}
