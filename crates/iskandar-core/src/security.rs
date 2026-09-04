//! iskandar-core :: security
//!
//! El resultado de auditar un sistema antes de conectarse a él.
//!
//! Dos invariantes que sostienen todo el diseño y que conviene que vivan en el
//! tipo, no en la disciplina de quien lo usa:
//!
//!   1. La severidad (`Severity`) dice QUÉ TAN GRAVE es un hallazgo.
//!      La disposición (`Disposition`) dice SI BLOQUEA la conexión.
//!      Son cosas distintas: algo puede ser `Critical` para el dueño y aun así
//!      `Informational` para Iskandar (su casa, su decisión), y algo puede ser
//!      `Medium` pero `Blocking` porque conectarme a eso me vuelve a MÍ
//!      superficie de ataque.
//!
//!   2. Un `Finding` carga una `Remediation` — la ruta que Iskandar le entrega
//!      al DUEÑO para que él cierre el hoyo. No existe, por diseño, ningún
//!      campo "exploit" ni "bypass". El hallazgo va hacia afuera como salida,
//!      nunca se usa como vía de entrada. La ética está en la forma del tipo.
//!
//! Nota de alcance: este módulo es el gate HACIA AFUERA (a qué me conecto).
//! El blindaje de Iskandar contra sí mismo (asumir que cada Firebird al otro
//! lado es hostil, aislamiento entre tenants, credenciales en reposo, el `.fbk`
//! envenenado) es otro concern y vive en otro lado. Primero eso, luego esto.
//!
//! v1: `SecurityAudit` es un módulo OPCIONAL de [`crate::ERPProvider`] (mismo
//! patrón que `clientes()`/`facturas()`), no un supertrait — un provider que
//! no aplica (p. ej. un futuro ERP cloud sin superficie de credenciales de
//! fábrica) simplemente no lo implementa y devuelve `None`. El audit nunca
//! corre por request: corre bajo demanda (`iskandar audit`) y una vez al
//! boot de `serve`, antes de levantar el listener. La política de v1 es
//! todo-o-nada a nivel proceso: si `gate()` de CUALQUIER provider configurado
//! da `Blocked`, el proceso completo se rehúsa a arrancar. No hay "modo
//! gated parcial" ni endpoint HTTP para el reporte en v1 — si algún día se
//! quiere exponer por API, es un cambio aparte.
//!
//! Waivers (`Reverify::OwnerWaiver`) quedan diferidos a v2: el tipo existe
//! (porque es parte de `Remediation`), pero no hay persistencia de waivers
//! todavía — en v1 todo hallazgo se re-verifica con `Reverify::ReRunCheck`.

use std::fmt;

use serde::Serialize;

/// Identificador estable de un tipo de hallazgo.
///
/// Estable a propósito: es lo que permite re-verificar tras la remediación.
/// El cliente dice "ya cerré el 3050", Iskandar vuelve a correr el audit, y
/// compara por `FindingId` para confirmar que el hallazgo desapareció antes de
/// habilitar `facturas` y compañía.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct FindingId(pub &'static str);

/// Qué tan grave es el hallazgo. No decide el gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Qué hace Iskandar con el hallazgo. Ortogonal a la severidad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Disposition {
    /// Conectarme a esto me vuelve superficie de ataque para mí y para todos
    /// mis otros tenants. Iskandar NO se conecta hasta que se remedie.
    Blocking,
    /// Es la casa del dueño. Se lo entrego en el reporte con severidad y
    /// remediación, pero no bloqueo. Él decide, bajo su riesgo.
    Informational,
}

/// La "otra solución": el mapa que Iskandar le da al dueño para cerrar el hoyo.
/// Siempre apunta hacia afuera (remediar), nunca hacia adentro (explotar).
#[derive(Debug, Clone, Serialize)]
pub struct Remediation {
    /// Qué cerrar, en una línea.
    pub summary: String,
    /// Pasos concretos que el dueño ejecuta en SU sistema.
    pub steps: Vec<String>,
    /// Cómo Iskandar confirma que de verdad se cerró antes de habilitar nada.
    pub reverify: Reverify,
}

/// Método con el que Iskandar re-verifica una remediación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Reverify {
    /// Vuelve a correr el mismo check; el hallazgo debe desaparecer.
    ReRunCheck,
    /// El dueño firma un waiver aceptando el riesgo. Solo válido para
    /// hallazgos `Informational`; un `Blocking` NUNCA se salta con firma.
    ///
    /// v1: la variante existe porque es parte del tipo, pero no hay
    /// persistencia de waivers todavía (nada de `iskandar.waivers.toml`) —
    /// eso queda diferido a v2. Ningún check de v1 produce esta variante.
    OwnerWaiver,
}

/// Un hallazgo de seguridad de un sistema auditado.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub id: FindingId,
    pub title: String,
    pub detail: String,
    pub severity: Severity,
    pub disposition: Disposition,
    pub remediation: Remediation,
    /// Evidencia de por qué se levantó el hallazgo (p. ej. "SYSDBA acepta
    /// masterkey", "3050 responde desde 0.0.0.0"). Opcional, para el reporte.
    pub evidence: Option<String>,
    /// Presente si el dueño firmó un waiver para este hallazgo (ver
    /// `Reverify::OwnerWaiver`). Puramente informativo para el reporte —
    /// `is_blocking()`/`gate()` NO lo consultan, así que un waiver
    /// estructuralmente no puede des-bloquear un `Disposition::Blocking`;
    /// la invariante del sketch original ("un Blocking NUNCA se salta con
    /// firma") queda garantizada por el tipo, no por disciplina de quien
    /// aplica los waivers.
    pub waived: Option<Waiver>,
}

impl Finding {
    pub fn is_blocking(&self) -> bool {
        matches!(self.disposition, Disposition::Blocking)
    }
}

/// Registro de que el dueño aceptó el riesgo de un hallazgo `Informational`
/// (persistido externamente, p. ej. `iskandar.waivers.toml` — ver
/// `iskandar-cli`; este tipo solo carga el dato, no sabe leer archivos).
#[derive(Debug, Clone, Serialize)]
pub struct Waiver {
    pub granted_at: String,
    pub note: String,
}

/// El reporte completo de un audit contra un provider.
#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub provider: &'static str,
    pub findings: Vec<Finding>,
}

impl AuditReport {
    pub fn blockers(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.is_blocking())
    }

    /// La pregunta que el core le hace al reporte antes de habilitar
    /// capacidades: ¿pasó el gate?
    pub fn gate(&self) -> GateOutcome {
        let blockers: Vec<FindingId> = self.blockers().map(|f| f.id).collect();
        if blockers.is_empty() {
            GateOutcome::Clear
        } else {
            GateOutcome::Blocked { blockers }
        }
    }
}

/// El veredicto del gate. Es lo único que el core necesita para decidir si
/// habilita `facturas`, `clientes`, etc. o si se planta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum GateOutcome {
    /// Sin bloqueantes. El lifecycle de conexión puede continuar.
    Clear,
    /// Hay bloqueantes. No se conecta. Se entrega la ruta de remediación y se
    /// espera a re-verificar.
    Blocked { blockers: Vec<FindingId> },
}

impl GateOutcome {
    pub fn is_clear(&self) -> bool {
        matches!(self, GateOutcome::Clear)
    }
}

/// Error al intentar auditar (no confundir con hallazgos: esto es que el audit
/// mismo no pudo correr, p. ej. no se pudo conectar para siquiera revisar).
#[derive(Debug)]
pub enum AuditError {
    Unreachable(String),
    ProviderError(String),
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditError::Unreachable(m) => write!(f, "no se pudo alcanzar el sistema para auditar: {m}"),
            AuditError::ProviderError(m) => write!(f, "el provider falló durante el audit: {m}"),
        }
    }
}

impl std::error::Error for AuditError {}

/// Capacidad de auto-auditarse. La implementa cada provider, no el core.
///
/// Cada provider conoce sus propios pecados: Microsip los de Firebird
/// (masterkey de fábrica, 3050 expuesto, `.fdb` sin cifrar, un solo usuario
/// todopoderoso, `.fbk` a la intemperie); CONTPAQi los suyos. El core solo
/// orquesta, corre el audit como precondición del lifecycle y le pregunta al
/// reporte por su `gate()`. No sabe NADA de Firebird — igual que no sabe nada
/// de `NewTrn`. La abstracción se valida por segunda vez aquí.
///
/// `async` por el mismo motivo que los demás módulos opcionales de
/// `ERPProvider` ([`crate::ClientesModule`], [`crate::FacturasModule`], ...):
/// providers cloud son naturalmente asíncronos, y providers síncronos como
/// Microsip envuelven el probe FFI en `spawn_blocking` para no bloquear el
/// runtime de axum.
#[async_trait::async_trait]
pub trait SecurityAudit: Send + Sync {
    async fn security_audit(&self) -> Result<AuditReport, AuditError>;
}
