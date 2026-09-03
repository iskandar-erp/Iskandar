//! El contrato central: [`ERPProvider`] y sus módulos de negocio.
//!
//! Cualquier ERP que quiera integrarse implementa este trait. El cliente
//! no sabe ni le importa si detrás hay una DLL, un protocolo propietario
//! o una base Firebird.
//!
//! El patrón de módulos opcionales es deliberado: no todos los ERPs
//! tienen todos los módulos. Un provider anuncia qué soporta regresando
//! `Some(...)` solo en los módulos que implementa.
//!
//! Los módulos son `async` porque los providers cloud (Siigo, Alegra)
//! son naturalmente asíncronos. Los providers síncronos (DLLs como
//! Microsip) envuelven su trabajo en `spawn_blocking`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{IskandarError, Result};
use crate::models::*;
use crate::security::SecurityAudit;

#[async_trait]
pub trait ERPProvider: Send + Sync {
    /// Nombre canónico del provider, p. ej. `"microsip"`. Es la clave
    /// en la configuración (`[providers.microsip]`) y en la URL
    /// (`/api/microsip/...`).
    fn name(&self) -> &'static str;

    /// Versión del provider (o de la API nativa del ERP, si se puede
    /// consultar).
    fn version(&self) -> String;

    /// Países donde opera el ERP subyacente.
    fn paises(&self) -> &[Pais];

    /// Prueba de conectividad real contra el ERP (cargar DLL, abrir
    /// sesión, hacer ping al API cloud...). Usada por `iskandar test`.
    async fn probar_conexion(&self) -> Result<()> {
        Err(IskandarError::NotImplemented("probar_conexion"))
    }

    // --- Módulos de negocio: exponer solo lo que el ERP soporta ---

    fn clientes(&self) -> Option<&dyn ClientesModule> {
        None
    }
    fn facturas(&self) -> Option<&dyn FacturasModule> {
        None
    }
    fn pedidos(&self) -> Option<&dyn PedidosModule> {
        None
    }
    fn inventario(&self) -> Option<&dyn InventarioModule> {
        None
    }
    fn compras(&self) -> Option<&dyn ComprasModule> {
        None
    }
    fn cxc(&self) -> Option<&dyn CxcModule> {
        None
    }
    fn contabilidad(&self) -> Option<&dyn ContabilidadModule> {
        None
    }

    /// Auditoría de seguridad hacia afuera (a qué sistema me conecto), no
    /// del propio Iskandar. Ver `iskandar_core::security` para el detalle
    /// de por qué es un módulo opcional y no un supertrait: un ERP que no
    /// tenga superficie de credenciales de fábrica (p. ej. un futuro
    /// provider cloud) simplemente no lo implementa.
    fn security(&self) -> Option<&dyn SecurityAudit> {
        None
    }

    /// Qué módulos anuncia este provider. Útil para discovery
    /// (`GET /api/providers`).
    fn capacidades(&self) -> Capacidades {
        Capacidades {
            clientes: self.clientes().is_some(),
            facturas: self.facturas().is_some(),
            pedidos: self.pedidos().is_some(),
            inventario: self.inventario().is_some(),
            compras: self.compras().is_some(),
            cxc: self.cxc().is_some(),
            contabilidad: self.contabilidad().is_some(),
            security: self.security().is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Capacidades {
    pub clientes: bool,
    pub facturas: bool,
    pub pedidos: bool,
    pub inventario: bool,
    pub compras: bool,
    pub cxc: bool,
    pub contabilidad: bool,
    pub security: bool,
}

#[async_trait]
pub trait ClientesModule: Send + Sync {
    async fn listar(&self, filtro: FiltroClientes) -> Result<Vec<Cliente>>;
    async fn obtener(&self, id: &EntidadId) -> Result<Cliente>;
    async fn crear(&self, cliente: NuevoCliente) -> Result<Cliente>;
}

#[async_trait]
pub trait FacturasModule: Send + Sync {
    async fn crear(&self, factura: NuevaFactura) -> Result<Factura>;
    async fn obtener(&self, id: &EntidadId) -> Result<Factura>;
}

#[async_trait]
pub trait PedidosModule: Send + Sync {
    async fn crear(&self, pedido: NuevoPedido) -> Result<Pedido>;
}

#[async_trait]
pub trait InventarioModule: Send + Sync {
    async fn articulos(&self, filtro: FiltroArticulos) -> Result<Vec<Articulo>>;
    async fn entrada(&self, entrada: NuevaEntrada) -> Result<DocumentoInventario>;
    async fn salida(&self, salida: NuevaSalida) -> Result<DocumentoInventario>;
}

#[async_trait]
pub trait ComprasModule: Send + Sync {
    async fn crear_orden(&self, orden: NuevaOrdenCompra) -> Result<OrdenCompra>;
    async fn crear_compra(&self, compra: NuevaCompra) -> Result<Compra>;
}

#[async_trait]
pub trait CxcModule: Send + Sync {
    async fn crear_credito(&self, credito: NuevoCredito) -> Result<Credito>;
}

#[async_trait]
pub trait ContabilidadModule: Send + Sync {
    async fn polizas(&self, filtro: FiltroPolizas) -> Result<Vec<Poliza>>;
}
