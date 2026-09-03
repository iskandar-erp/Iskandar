//! CLI de Iskandar.
//!
//! ```text
//! iskandar init --provider microsip --dll-path "C:\Microsip\ApiMicrosip.dll"
//! iskandar serve --port 8080
//! iskandar test --provider microsip
//! ```

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

use iskandar_api::ApiState;
use iskandar_core::{AuditReport, Finding, ProviderConfig, ProviderRegistry};

#[derive(Parser)]
#[command(
    name = "iskandar",
    version,
    about = "Integración open source de ERPs para América Latina"
)]
struct Cli {
    /// Archivo de configuración TOML.
    #[arg(long, global = true, default_value = "iskandar.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Genera un archivo de configuración inicial.
    Init {
        #[arg(long, default_value = "microsip")]
        provider: String,
        #[arg(long)]
        dll_path: Option<String>,
        #[arg(long, default_value = "SYSDBA")]
        usuario: String,
        /// Password de la base de datos. Requerido: `init` ya no genera un
        /// default inseguro (antes escribía la contraseña SYSDBA/masterkey
        /// documentada de Firebird).
        #[arg(long)]
        password: String,
    },
    /// Levanta el servidor HTTP con los providers configurados.
    Serve {
        /// Puerto; si se omite, usa el de la configuración (default 8080).
        #[arg(long)]
        port: Option<u16>,
    },
    /// Prueba la conexión de un provider configurado.
    Test {
        #[arg(long)]
        provider: String,
    },
    /// Audita al sistema al que se conecta un provider (gate hacia
    /// afuera, no una auditoría de Iskandar mismo). Exit code 1 si el
    /// reporte queda `Blocked` (hay hallazgos bloqueantes), 0 si `Clear`.
    Audit {
        #[arg(long)]
        provider: String,
        /// Imprime el reporte completo como JSON en vez del formato
        /// legible agrupado por severidad/disposición.
        #[arg(long)]
        json: bool,
    },
    /// Inspecciona el esquema de la base de datos del ERP.
    ///
    /// Sin --tabla lista todas las tablas; con --tabla describe sus columnas.
    Schema {
        #[arg(long)]
        provider: String,
        /// Nombre de la tabla a describir (p. ej. CLIENTES). Opcional.
        #[arg(long)]
        tabla: Option<String>,
        /// Junto con --tabla: en vez de describir columnas, lista los
        /// valores distintos que existen para este campo (útil para
        /// descifrar campos tipo código, p. ej. TIPO_DOCTO).
        #[arg(long)]
        valores: Option<String>,
        /// Junto con --tabla: imprime las primeras N filas reales (todas
        /// las columnas, como texto). Útil para ver el formato exacto en
        /// que la DLL devuelve fechas/decimales antes de parsearlos.
        #[arg(long)]
        muestra: Option<u32>,
    },
}

#[derive(Debug, Default, Deserialize)]
struct AppConfig {
    #[serde(default)]
    server: ServerConfig,
    #[serde(default)]
    providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[serde(default = "puerto_default")]
    port: u16,
    /// Token que deben presentar los clientes en `Authorization: Bearer
    /// <token>`. También puede darse por la variable de entorno
    /// `ISKANDAR_API_TOKEN` (tiene prioridad sobre este valor). El servidor
    /// se niega a arrancar si no hay token por ninguna de las dos vías.
    #[serde(default)]
    token: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: puerto_default(),
            token: None,
        }
    }
}

fn puerto_default() -> u16 {
    8080
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Init { provider, dll_path, usuario, password } => {
            init(&cli.config, &provider, dll_path.as_deref(), &usuario, &password)
        }
        Command::Serve { port } => serve(&cli.config, port).await,
        Command::Test { provider } => test(&cli.config, &provider).await,
        Command::Audit { provider, json } => audit(&cli.config, &provider, json).await,
        Command::Schema { provider, tabla, valores, muestra } => {
            schema(&cli.config, &provider, tabla.as_deref(), valores.as_deref(), muestra).await
        }
    }
}

/// Providers compilados en este binario.
fn build_registry() -> ProviderRegistry {
    #[allow(unused_mut)]
    let mut registry = ProviderRegistry::new();
    #[cfg(windows)]
    registry.register(
        "microsip",
        Box::new(iskandar_microsip::MicrosipProvider::from_provider_config),
    );
    registry
}

fn load_config(path: &Path) -> Result<AppConfig, Box<dyn Error>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("no se pudo leer '{}': {e}", path.display()))?;
    Ok(toml::from_str(&raw)?)
}

/// Escapa un valor para insertarlo en un TOML basic string (comillas dobles).
fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn init(
    path: &Path,
    provider: &str,
    dll_path: Option<&str>,
    usuario: &str,
    password: &str,
) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!(
            "'{}' ya existe — edítalo directamente o bórralo primero",
            path.display()
        )
        .into());
    }
    let dll = dll_path.unwrap_or(r"C:\Microsip\ApiMicrosip.dll");
    let usuario = toml_escape(usuario);
    let password = toml_escape(password);
    let contents = format!(
        r#"# Configuración de Iskandar — generada por `iskandar init`
[server]
port = 8080
# Requerido para levantar el servidor: token que deben presentar los
# clientes en `Authorization: Bearer <token>`. Puedes darlo aquí o por la
# variable de entorno ISKANDAR_API_TOKEN (tiene prioridad sobre este valor).
# token = "reemplaza-esto-por-un-token-largo-y-aleatorio"

[providers.{provider}]
dll_path = '{dll}'
db_path = 'C:\Microsip datos\EMPRESA.FDB'
usuario = "{usuario}"
password = "{password}"
existencias_negativas = true
validar_precio_minimo = false
"#
    );
    std::fs::write(path, contents)?;
    println!("Configuración creada en '{}'. Ajusta la ruta de la base de datos antes de usarla.", path.display());
    Ok(())
}

async fn serve(config_path: &Path, port: Option<u16>) -> Result<(), Box<dyn Error>> {
    let config = load_config(config_path)?;
    let registry = build_registry();

    let mut providers = HashMap::new();
    for (name, provider_config) in &config.providers {
        match registry.create(name, provider_config) {
            Ok(provider) => {
                tracing::info!(provider = %name, "provider inicializado");

                // Gate hacia afuera: auditar al sistema al que este provider
                // se conecta ANTES de aceptar tráfico. Providers que no
                // implementan `SecurityAudit` (devuelven `None`) se saltan
                // el check — no es un error, simplemente no aplica.
                if let Some(security) = provider.security() {
                    match security.security_audit().await {
                        Ok(report) if report.gate().is_clear() => {
                            tracing::info!(provider = %name, "security audit: gate CLEAR");
                        }
                        Ok(report) => {
                            eprintln!(
                                "=== iskandar: el provider '{name}' tiene hallazgos de \
                                 seguridad BLOQUEANTES — el servidor no arrancará ===\n"
                            );
                            eprintln!("{}", formatear_reporte(&report));
                            return Err(format!(
                                "provider '{name}': gate de seguridad BLOQUEADO — remedia \
                                 los hallazgos de arriba y vuelve a intentar (ver \
                                 `iskandar audit --provider {name}` para el detalle)"
                            )
                            .into());
                        }
                        Err(e) => {
                            return Err(format!(
                                "no se pudo auditar la seguridad del provider '{name}' \
                                 antes de arrancar: {e}"
                            )
                            .into());
                        }
                    }
                }

                providers.insert(name.clone(), provider);
            }
            Err(e) => {
                tracing::error!(provider = %name, error = %e, "no se pudo inicializar el provider");
            }
        }
    }
    if providers.is_empty() {
        tracing::warn!(
            "ningún provider activo — revisa las secciones [providers.*] de '{}'",
            config_path.display()
        );
    }

    let api_token = std::env::var("ISKANDAR_API_TOKEN")
        .ok()
        .or_else(|| config.server.token.clone())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            "no se configuró un token de autenticación para la API — define la variable de \
             entorno ISKANDAR_API_TOKEN o '[server].token' en la configuración antes de \
             levantar el servidor"
                .to_string()
        })?;

    let port = port.unwrap_or(config.server.port);
    let state = Arc::new(ApiState::new(providers, api_token));
    let app = iskandar_api::router(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("Iskandar escuchando en http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn schema(
    config_path: &Path,
    provider_name: &str,
    tabla: Option<&str>,
    valores: Option<&str>,
    muestra: Option<u32>,
) -> Result<(), Box<dyn Error>> {
    #[cfg(not(windows))]
    {
        let _ = (config_path, provider_name, tabla, valores, muestra);
        return Err("el subcomando schema solo está disponible en Windows (requiere ApiMicrosip.dll)".into());
    }

    #[cfg(windows)]
    {
        let config = load_config(config_path)?;
        let provider_config = config.providers.get(provider_name).ok_or_else(|| {
            format!(
                "no hay sección [providers.{provider_name}] en '{}'",
                config_path.display()
            )
        })?;

        if provider_name != "microsip" {
            return Err(format!("schema solo soporta el provider 'microsip' por ahora (recibido: '{provider_name}')").into());
        }

        let microsip_config = provider_config.typed::<iskandar_microsip::MicrosipConfig>()?;
        let provider = iskandar_microsip::MicrosipProvider::new(microsip_config)?;

        match tabla {
            None => {
                let tablas = provider.listar_tablas().await?;
                println!("{} tablas encontradas:\n", tablas.len());
                for t in &tablas {
                    println!("  {t}");
                }
            }
            Some(nombre_tabla) if valores.is_some() => {
                let campo = valores.unwrap();
                let vals = provider.valores_distintos(nombre_tabla, campo).await?;
                println!(
                    "Valores distintos de {}.{} ({}):\n",
                    nombre_tabla.to_uppercase(),
                    campo.to_uppercase(),
                    vals.len()
                );
                for v in &vals {
                    println!("  {v:?}");
                }
            }
            Some(nombre_tabla) if muestra.is_some() => {
                let limite = muestra.unwrap();
                let filas = provider.muestra_tabla(nombre_tabla, limite).await?;
                println!("{} fila(s) de muestra de {}:\n", filas.len(), nombre_tabla.to_uppercase());
                for (i, fila) in filas.iter().enumerate() {
                    println!("--- fila {} ---", i + 1);
                    for (campo, valor) in fila {
                        println!("  {campo:<30} {valor:?}");
                    }
                    println!();
                }
            }
            Some(nombre_tabla) => {
                let campos = provider.describir_tabla(nombre_tabla).await?;
                if campos.is_empty() {
                    println!("La tabla '{nombre_tabla}' no existe o no tiene columnas.");
                    return Ok(());
                }
                println!("Tabla: {}\n", nombre_tabla.to_uppercase());
                println!("  {:<35} {}", "CAMPO", "TIPO");
                println!("  {}", "-".repeat(55));
                for c in &campos {
                    println!("  {:<35} {}", c.nombre, c.tipo);
                }
            }
        }
        Ok(())
    }
}

async fn test(config_path: &Path, provider_name: &str) -> Result<(), Box<dyn Error>> {
    let config = load_config(config_path)?;
    let provider_config = config.providers.get(provider_name).ok_or_else(|| {
        format!(
            "no hay sección [providers.{provider_name}] en '{}'",
            config_path.display()
        )
    })?;

    let registry = build_registry();
    let provider = registry.create(provider_name, provider_config)?;

    println!("provider:    {} v{}", provider.name(), provider.version());
    println!(
        "capacidades: {}",
        serde_json::to_string(&provider.capacidades())?
    );

    print!("conexión:    ");
    match provider.probar_conexion().await {
        Ok(()) => println!("OK"),
        Err(e) => {
            println!("FALLÓ — {e}");
            return Err(e.into());
        }
    }
    Ok(())
}

/// `iskandar audit --provider <nombre> [--json]`.
///
/// Corre el gate hacia afuera bajo demanda (fuera del boot de `serve`).
/// Exit code: 0 si el reporte queda `Clear`, 1 si queda `Blocked` (hallazgos
/// bloqueantes) o si el audit mismo no pudo correr.
async fn audit(config_path: &Path, provider_name: &str, json: bool) -> Result<(), Box<dyn Error>> {
    let config = load_config(config_path)?;
    let provider_config = config.providers.get(provider_name).ok_or_else(|| {
        format!(
            "no hay sección [providers.{provider_name}] en '{}'",
            config_path.display()
        )
    })?;

    let registry = build_registry();
    let provider = registry.create(provider_name, provider_config)?;

    let security = provider.security().ok_or_else(|| {
        format!(
            "el provider '{provider_name}' no implementa auditoría de seguridad \
             (SecurityAudit) — no hay nada que auditar"
        )
    })?;

    let report = security.security_audit().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", formatear_reporte(&report));
    }

    if !report.gate().is_clear() {
        std::process::exit(1);
    }
    Ok(())
}

/// Formato legible del reporte, agrupado por severidad (más grave primero)
/// y con la remediación completa de cada hallazgo. Usado tanto por
/// `iskandar audit` (stdout) como por el gate de `serve` (stderr).
fn formatear_reporte(report: &AuditReport) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "provider:  {}", report.provider);
    let _ = writeln!(out, "hallazgos: {}", report.findings.len());
    let _ = writeln!(out);

    if report.findings.is_empty() {
        let _ = writeln!(out, "(sin hallazgos)");
    }

    // Bloqueantes primero, luego por severidad descendente — es el orden
    // en que a un operador le urge leerlos.
    let mut ordenados: Vec<&Finding> = report.findings.iter().collect();
    ordenados.sort_by_key(|f| (!f.is_blocking(), std::cmp::Reverse(f.severity)));

    for f in ordenados {
        let disp = if f.is_blocking() { "BLOCKING" } else { "informational" };
        let _ = writeln!(out, "[{:?} / {disp}] {} ({})", f.severity, f.title, f.id.0);
        let _ = writeln!(out, "  {}", f.detail);
        if let Some(ev) = &f.evidence {
            let _ = writeln!(out, "  evidencia: {ev}");
        }
        let _ = writeln!(out, "  remediación: {}", f.remediation.summary);
        for (i, paso) in f.remediation.steps.iter().enumerate() {
            let _ = writeln!(out, "    {}. {paso}", i + 1);
        }
        let _ = writeln!(out, "  re-verificación: {:?}", f.remediation.reverify);
        let _ = writeln!(out);
    }

    match report.gate() {
        iskandar_core::GateOutcome::Clear => {
            let _ = writeln!(out, "gate: CLEAR");
        }
        iskandar_core::GateOutcome::Blocked { blockers } => {
            let ids: Vec<&str> = blockers.iter().map(|b| b.0).collect();
            let _ = writeln!(out, "gate: BLOCKED — bloqueantes: {ids:?}");
        }
    }

    out
}
