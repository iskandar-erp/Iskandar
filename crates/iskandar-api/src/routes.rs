//! Rutas REST universales sobre los providers activos.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use iskandar_core::models::*;
use iskandar_core::{ERPClient, ERPProvider, IskandarError};

/// Estado compartido del servidor: providers activos por nombre y el token
/// que autoriza las rutas `/api/*`. `/health` queda fuera de la protección.
pub struct ApiState {
    providers: HashMap<String, Arc<dyn ERPProvider>>,
    api_token: String,
}

impl ApiState {
    pub fn new(providers: HashMap<String, Arc<dyn ERPProvider>>, api_token: String) -> Self {
        Self {
            providers,
            api_token,
        }
    }

    fn client(&self, nombre: &str) -> Result<ERPClient, ApiError> {
        self.providers
            .get(nombre)
            .cloned()
            .map(ERPClient::new)
            .ok_or_else(|| ApiError(IskandarError::UnknownProvider(nombre.to_string())))
    }
}

pub fn router(state: Arc<ApiState>) -> Router {
    let api_routes = Router::new()
        .route("/api/providers", get(listar_providers))
        .route("/api/{provider}/clientes", get(listar_clientes))
        .route("/api/{provider}/clientes/{id}", get(obtener_cliente))
        .route("/api/{provider}/facturas", post(crear_factura))
        .route("/api/{provider}/facturas/{id}", get(obtener_factura))
        .route("/api/{provider}/inventario/articulos", get(listar_articulos))
        .route("/api/{provider}/cxc/creditos", post(crear_credito))
        // route_layer solo protege las rutas registradas arriba en este
        // Router (no /health, que se agrega después sin el middleware).
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/health", get(health))
        .merge(api_routes)
        .with_state(state)
}

/// Exige `Authorization: Bearer <token>` en cada request a `/api/*`.
/// Comparación en tiempo constante para no filtrar el token por timing.
async fn require_token(
    State(state): State<Arc<ApiState>>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match presented {
        Some(token) if constant_time_eq(token.as_bytes(), state.api_token.as_bytes()) => {
            next.run(request).await
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        )
            .into_response(),
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

/// Discovery: qué providers están activos y qué módulos anuncian.
async fn listar_providers(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let info: Vec<_> = state
        .providers
        .values()
        .map(|p| {
            json!({
                "name": p.name(),
                "version": p.version(),
                "paises": p.paises(),
                "capacidades": p.capacidades(),
            })
        })
        .collect();
    Json(info)
}

async fn listar_clientes(
    State(state): State<Arc<ApiState>>,
    Path(provider): Path<String>,
    Query(filtro): Query<FiltroClientes>,
) -> Result<Json<Vec<Cliente>>, ApiError> {
    let client = state.client(&provider)?;
    Ok(Json(client.clientes()?.listar(filtro).await?))
}

async fn obtener_cliente(
    State(state): State<Arc<ApiState>>,
    Path((provider, id)): Path<(String, String)>,
) -> Result<Json<Cliente>, ApiError> {
    let client = state.client(&provider)?;
    let id = EntidadId::parse(&id);
    Ok(Json(client.clientes()?.obtener(&id).await?))
}

async fn crear_factura(
    State(state): State<Arc<ApiState>>,
    Path(provider): Path<String>,
    Json(factura): Json<NuevaFactura>,
) -> Result<(StatusCode, Json<Factura>), ApiError> {
    let client = state.client(&provider)?;
    let creada = client.facturas()?.crear(factura).await?;
    Ok((StatusCode::CREATED, Json(creada)))
}

async fn obtener_factura(
    State(state): State<Arc<ApiState>>,
    Path((provider, id)): Path<(String, String)>,
) -> Result<Json<Factura>, ApiError> {
    let client = state.client(&provider)?;
    let id = EntidadId::parse(&id);
    Ok(Json(client.facturas()?.obtener(&id).await?))
}

async fn listar_articulos(
    State(state): State<Arc<ApiState>>,
    Path(provider): Path<String>,
    Query(filtro): Query<FiltroArticulos>,
) -> Result<Json<Vec<Articulo>>, ApiError> {
    let client = state.client(&provider)?;
    Ok(Json(client.inventario()?.articulos(filtro).await?))
}

async fn crear_credito(
    State(state): State<Arc<ApiState>>,
    Path(provider): Path<String>,
    Json(credito): Json<NuevoCredito>,
) -> Result<(StatusCode, Json<Credito>), ApiError> {
    let client = state.client(&provider)?;
    let creado = client.cxc()?.crear_credito(credito).await?;
    Ok((StatusCode::CREATED, Json(creado)))
}

/// Traducción de errores del framework a HTTP.
struct ApiError(IskandarError);

impl From<IskandarError> for ApiError {
    fn from(e: IskandarError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            IskandarError::UnknownProvider(_) | IskandarError::NotFound(_) => {
                StatusCode::NOT_FOUND
            }
            IskandarError::UnsupportedModule { .. } | IskandarError::NotImplemented(_) => {
                StatusCode::NOT_IMPLEMENTED
            }
            IskandarError::Validation(_) | IskandarError::Config(_) => StatusCode::BAD_REQUEST,
            IskandarError::Connection(_)
            | IskandarError::Provider { .. }
            | IskandarError::Io(_) => StatusCode::BAD_GATEWAY,
        };
        (status, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}
