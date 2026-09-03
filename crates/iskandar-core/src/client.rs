//! [`ERPClient`]: la puerta de entrada universal.
//!
//! Envuelve un provider y convierte "módulo no disponible" en un error
//! tipado en lugar de un `Option` que cada llamador tendría que manejar.

use std::sync::Arc;

use crate::error::{IskandarError, Result};
use crate::provider::*;

#[derive(Clone)]
pub struct ERPClient {
    provider: Arc<dyn ERPProvider>,
}

impl ERPClient {
    pub fn new(provider: Arc<dyn ERPProvider>) -> Self {
        Self { provider }
    }

    /// Acceso directo al provider subyacente (nombre, versión,
    /// capacidades, prueba de conexión).
    pub fn provider(&self) -> &dyn ERPProvider {
        self.provider.as_ref()
    }

    fn modulo<'a, M: ?Sized>(&self, modulo: Option<&'a M>, nombre: &str) -> Result<&'a M> {
        modulo.ok_or_else(|| IskandarError::UnsupportedModule {
            provider: self.provider.name().to_string(),
            module: nombre.to_string(),
        })
    }

    pub fn clientes(&self) -> Result<&dyn ClientesModule> {
        self.modulo(self.provider.clientes(), "clientes")
    }

    pub fn facturas(&self) -> Result<&dyn FacturasModule> {
        self.modulo(self.provider.facturas(), "facturas")
    }

    pub fn pedidos(&self) -> Result<&dyn PedidosModule> {
        self.modulo(self.provider.pedidos(), "pedidos")
    }

    pub fn inventario(&self) -> Result<&dyn InventarioModule> {
        self.modulo(self.provider.inventario(), "inventario")
    }

    pub fn compras(&self) -> Result<&dyn ComprasModule> {
        self.modulo(self.provider.compras(), "compras")
    }

    pub fn cxc(&self) -> Result<&dyn CxcModule> {
        self.modulo(self.provider.cxc(), "cxc")
    }

    pub fn contabilidad(&self) -> Result<&dyn ContabilidadModule> {
        self.modulo(self.provider.contabilidad(), "contabilidad")
    }

    pub fn security(&self) -> Result<&dyn crate::security::SecurityAudit> {
        self.modulo(self.provider.security(), "security")
    }
}
