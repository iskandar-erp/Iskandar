//! Implementación de `SecurityAudit` para Microsip.
//!
//! v1: dos hallazgos, ambos reusando `MicrosipDll::connect()`/`disconnect()`
//! ya validados en vivo (`dll.rs`) — sin FFI nueva.
//!
//! - `F-MSIP-FACTORY-CREDS-ACTIVE`: probe activo. Intenta conectar al
//!   Firebird REAL configurado (`config.db_path`) con la credencial de
//!   fábrica `SYSDBA`/`masterkey`, sin importar qué credencial tenga
//!   configurada Iskandar. Si conecta, el Firebird de destino sigue
//!   aceptando la contraseña de fábrica — `Critical` + `Blocking`.
//! - `F-MSIP-FACTORY-CREDS-CONFIG`: chequeo estático, señal débil
//!   complementaria. Si la config LOCAL de Iskandar también usa
//!   `SYSDBA`/`masterkey`, se informa (no bloquea) — es una pista, no una
//!   prueba: Firebird trunca la contraseña de SYSDBA a los primeros 8
//!   caracteres, así que comparar el string completo es débil por diseño;
//!   la señal fuerte es el probe activo de arriba.
//!
//! ## Explícitamente fuera de alcance v1
//! (ver sesión de planeación con `iskandar-planificador`; no implementar
//! nada de esto, ni como TODO parcial):
//! - Puerto 3050 expuesto a red: requeriría perspectiva de red externa
//!   (escaneo desde fuera del host), que Iskandar no tiene en v1.
//! - `.fdb` sin cifrar en disco: requeriría acceso al filesystem remoto del
//!   servidor Firebird, que Iskandar no tiene (solo habla el protocolo de
//!   la API/DLL).
//! - Enumeración de usuarios Firebird / "un solo super-usuario": requeriría
//!   FFI nueva contra el Services API de Firebird, fuera del alcance de
//!   "sin FFI nueva" de este v1.

use std::sync::Arc;

use async_trait::async_trait;
use iskandar_core::{
    AuditError, AuditReport, Disposition, Finding, FindingId, IskandarError, Remediation,
    Reverify, SecurityAudit, Severity,
};

use crate::dll::MicrosipDll;
use crate::models::MicrosipConfig;
use crate::provider::run_blocking;

const F_FACTORY_CREDS_ACTIVE: FindingId = FindingId("F-MSIP-FACTORY-CREDS-ACTIVE");
const F_FACTORY_CREDS_CONFIG: FindingId = FindingId("F-MSIP-FACTORY-CREDS-CONFIG");

/// Credencial de fábrica de Firebird, documentada públicamente por Firebird
/// mismo (no es un secreto de Microsip). El probe activo es la señal fuerte
/// precisamente porque este string es de conocimiento público.
const SYSDBA_USUARIO: &str = "SYSDBA";
const SYSDBA_PASSWORD_FABRICA: &str = "masterkey";

pub(crate) struct SecurityMicrosip {
    pub(crate) dll: Arc<MicrosipDll>,
    pub(crate) config: MicrosipConfig,
}

#[async_trait]
impl SecurityAudit for SecurityMicrosip {
    async fn security_audit(&self) -> Result<AuditReport, AuditError> {
        let mut findings = Vec::new();

        if config_usa_credenciales_de_fabrica(&self.config) {
            findings.push(finding_factory_creds_config());
        }

        if probe_factory_creds_activo(&self.dll, &self.config).await? {
            findings.push(finding_factory_creds_active(&self.config.db_path));
        }

        Ok(AuditReport { provider: "microsip", findings })
    }
}

/// Señal débil: ¿la config LOCAL de Iskandar (no el Firebird de destino)
/// también usa la pareja de fábrica? Comparación literal de `password`
/// (sin truncar a 8 caracteres) porque esto es solo una pista adicional,
/// no la prueba — la prueba real es `probe_factory_creds_activo`.
fn config_usa_credenciales_de_fabrica(config: &MicrosipConfig) -> bool {
    config.usuario.eq_ignore_ascii_case(SYSDBA_USUARIO) && config.password == SYSDBA_PASSWORD_FABRICA
}

/// Probe activo: intenta conectar al Firebird real con la credencial de
/// fábrica, sin importar qué credencial tenga configurada Iskandar. Cierra
/// la conexión de inmediato, éxito o no.
///
/// Distingue dos motivos de fallo de `dll.connect()`:
/// - `IskandarError::Provider { .. }`: `DBConnect` corrió y Firebird
///   respondió limpio con un `ErrCode` de rechazo (según `check()` en
///   `dll.rs`, que convierte cualquier código != 0 en este variant). Es la
///   respuesta ESPERADA cuando la credencial de fábrica no funciona —
///   `Ok(false)`, no es un error del audit.
/// - Cualquier otro `IskandarError` (p. ej. `Connection`, si `NewDB`/`NewTrn`
///   fallan antes de siquiera intentar `DBConnect`, o el símbolo no existe):
///   el probe no pudo correr, no es una respuesta sobre la credencial —
///   se propaga como `AuditError::Unreachable`.
///
/// VALIDADO EN VIVO (2026-09-03) contra `ApiMicrosip2025.dll` y un Firebird
/// real (`TEST_PRUEBAS.FDB`, target `i686-pc-windows-msvc`):
/// - Credencial de fábrica correcta → `connect()` tiene éxito, `disconnect()`
///   limpio, sin colgarse.
/// - Credencial incorrecta → `DBConnect` regresa en ~0.24s con
///   `IskandarError::Provider { code: 4, message: "...El nombre de usuario o
///   la contraseña son inválidos..." }` — cae en la rama esperada, sin
///   `panic!`, sin colgar el proceso ni dejar transacción/sesión huérfana.
async fn probe_factory_creds_activo(
    dll: &Arc<MicrosipDll>,
    config: &MicrosipConfig,
) -> Result<bool, AuditError> {
    let dll = dll.clone();
    let db_path = config.db_path.clone();

    run_blocking(move || match dll.connect(&db_path, SYSDBA_USUARIO, SYSDBA_PASSWORD_FABRICA) {
        Ok(handle) => {
            dll.disconnect(handle).ok();
            Ok(true)
        }
        Err(IskandarError::Provider { .. }) => Ok(false),
        Err(e) => Err(e),
    })
    .await
    .map_err(|e| AuditError::Unreachable(e.to_string()))
}

fn finding_factory_creds_active(db_path: &str) -> Finding {
    Finding {
        id: F_FACTORY_CREDS_ACTIVE,
        title: "El Firebird de destino acepta la contraseña de fábrica de SYSDBA".into(),
        detail: "Se intentó conectar al Firebird configurado usando la credencial de \
                 fábrica SYSDBA/masterkey (documentada públicamente por Firebird, no un \
                 secreto de Microsip) y la conexión tuvo éxito. Cualquiera con acceso de \
                 red al puerto de Firebird puede autenticarse como superusuario sin \
                 conocer ninguna credencial real, sin importar qué credencial tenga \
                 configurada Iskandar. Conectarse a un sistema así vuelve a Iskandar (y a \
                 sus demás tenants) superficie de ataque."
            .into(),
        severity: Severity::Critical,
        disposition: Disposition::Blocking,
        remediation: Remediation {
            summary: "Cambiar la contraseña de SYSDBA en el Firebird de destino".into(),
            steps: vec![
                "Detener temporalmente el acceso de red al servicio de Firebird si es \
                 posible mientras se hace el cambio."
                    .into(),
                "Cambiar la contraseña de SYSDBA con `gsec` (o el Server Manager de \
                 Firebird) por una contraseña fuerte — recordar que Firebird trunca la \
                 contraseña de SYSDBA a los primeros 8 caracteres, así que los primeros \
                 8 caracteres por sí solos deben ser suficientemente fuertes."
                    .into(),
                "Actualizar `[providers.microsip].password` en la configuración de \
                 Iskandar con la nueva contraseña."
                    .into(),
                "Volver a correr `iskandar audit --provider microsip` y confirmar que \
                 este hallazgo desaparece."
                    .into(),
            ],
            reverify: Reverify::ReRunCheck,
        },
        evidence: Some(format!("SYSDBA acepta la contraseña de fábrica en {db_path}")),
    }
}

fn finding_factory_creds_config() -> Finding {
    Finding {
        id: F_FACTORY_CREDS_CONFIG,
        title: "La configuración local de Iskandar también usa la pareja de fábrica".into(),
        detail: "`[providers.microsip].usuario`/`password` en la configuración de Iskandar \
                 coinciden con la pareja de fábrica SYSDBA/masterkey. Esto es una señal \
                 débil (Firebird trunca la contraseña de SYSDBA a 8 caracteres, así que \
                 esta comparación literal no prueba nada sobre el Firebird de destino), \
                 pero indica que Iskandar mismo no está usando una credencial dedicada."
            .into(),
        severity: Severity::High,
        disposition: Disposition::Informational,
        remediation: Remediation {
            summary: "Usar una credencial de aplicación dedicada, no SYSDBA/masterkey".into(),
            steps: vec![
                "Crear un usuario de Firebird dedicado para Iskandar (no SYSDBA) con solo \
                 los permisos que necesita."
                    .into(),
                "Actualizar `[providers.microsip].usuario`/`password` en la configuración \
                 de Iskandar."
                    .into(),
            ],
            reverify: Reverify::ReRunCheck,
        },
        evidence: Some(
            "MicrosipConfig.usuario == 'SYSDBA' (case-insensitive) y password == \
             'masterkey' (comparación literal, sin truncar)"
                .into(),
        ),
    }
}
