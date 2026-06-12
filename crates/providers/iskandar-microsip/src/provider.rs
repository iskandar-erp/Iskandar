//! Implementación del trait `ERPProvider` para Microsip.
//!
//! La DLL es síncrona y no thread-safe, así que cada operación corre en
//! `spawn_blocking` y `MicrosipDll` serializa el acceso con su `Mutex`.

use std::sync::Arc;

use async_trait::async_trait;
use iskandar_core::models::*;
use iskandar_core::{
    ERPProvider, FacturasModule, IskandarError, ProviderConfig, Result,
};

use crate::dll::MicrosipDll;
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
    facturas: FacturasMicrosip,
}

impl MicrosipProvider {
    /// Carga la DLL y deja el provider listo. La conexión a la base se
    /// abre por operación mientras definimos el manejo de sesión.
    pub fn new(config: MicrosipConfig) -> Result<Self> {
        let dll = Arc::new(MicrosipDll::load(&config.dll_path)?);
        let facturas = FacturasMicrosip {
            dll: dll.clone(),
            config: config.clone(),
        };
        Ok(Self {
            config,
            dll,
            facturas,
        })
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
            dll.disconnect(handle)
        })
        .await
    }

    fn facturas(&self) -> Option<&dyn FacturasModule> {
        Some(&self.facturas)
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
        let _ = id;
        // Lectura: irá por SQL de la API Básica (Dataset/Sql) sobre
        // DOCTOS_VE, no por las funciones de captura.
        Err(IskandarError::NotImplemented("microsip::facturas::obtener"))
    }
}

/// Corre trabajo síncrono de la DLL fuera del executor async.
async fn run_blocking<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| IskandarError::Connection(format!("tarea bloqueante falló: {e}")))?
}
