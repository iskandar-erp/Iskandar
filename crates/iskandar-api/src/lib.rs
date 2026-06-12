//! # iskandar-api
//!
//! Capa HTTP opcional de Iskandar. El core funciona como librería sin
//! este crate; esto solo expone los providers activos como REST.
//!
//! Un mismo proceso sirve múltiples providers: el provider se
//! selecciona en la URL (`/api/{provider}/...`).

mod routes;

pub use routes::{router, ApiState};
