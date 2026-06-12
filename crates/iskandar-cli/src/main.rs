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
use iskandar_core::{ProviderConfig, ProviderRegistry};

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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: puerto_default(),
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
        Command::Init { provider, dll_path } => init(&cli.config, &provider, dll_path.as_deref()),
        Command::Serve { port } => serve(&cli.config, port).await,
        Command::Test { provider } => test(&cli.config, &provider).await,
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

fn init(path: &Path, provider: &str, dll_path: Option<&str>) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!(
            "'{}' ya existe — edítalo directamente o bórralo primero",
            path.display()
        )
        .into());
    }
    let dll = dll_path.unwrap_or(r"C:\Microsip\ApiMicrosip.dll");
    let contents = format!(
        r#"# Configuración de Iskandar — generada por `iskandar init`
[server]
port = 8080

[providers.{provider}]
dll_path = '{dll}'
db_path = 'C:\Microsip datos\EMPRESA.FDB'
usuario = "SYSDBA"
password = "masterkey"
existencias_negativas = true
validar_precio_minimo = false
"#
    );
    std::fs::write(path, contents)?;
    println!("Configuración creada en '{}'. Ajusta credenciales y rutas antes de usarla.", path.display());
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

    let port = port.unwrap_or(config.server.port);
    let state = Arc::new(ApiState::new(providers));
    let app = iskandar_api::router(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("Iskandar escuchando en http://0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
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
