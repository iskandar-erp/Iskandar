# Contribuir a Iskandar

> *El mismo fuego, traducido.*

Gracias por estar aquí. Iskandar existe porque el open source nos dio
herramientas cuando no podíamos pagar licencias — y la única forma de
devolver ese favor es construyendo juntos. Toda contribución cuenta:
código, documentación, reportes de bugs, o simplemente probar el
framework contra tu ERP y contarnos qué pasó.

---

## La contribución más valiosa: un provider nuevo

Iskandar crece un ERP a la vez. Si tienes acceso a un ERP que no está en
la lista (o conoces uno mejor que nosotros), tienes todo lo necesario
para construir la siguiente pieza.

### Cómo funciona la arquitectura

Cada provider es un **crate independiente** bajo `crates/providers/`.
Tus dependencias no contaminan al resto: si tu ERP necesita un cliente
REST, una DLL de Windows o un driver de base de datos, eso vive solo en
tu crate. El contrato central es el trait [`ERPProvider`]
(`crates/iskandar-core/src/provider.rs`):

```rust
#[async_trait]
pub trait ERPProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> String;
    fn paises(&self) -> &[Pais];
    async fn probar_conexion(&self) -> Result<()>;

    // Módulos opcionales — expón SOLO lo que tu ERP soporta
    fn clientes(&self) -> Option<&dyn ClientesModule> { None }
    fn facturas(&self) -> Option<&dyn FacturasModule> { None }
    fn pedidos(&self) -> Option<&dyn PedidosModule> { None }
    fn inventario(&self) -> Option<&dyn InventarioModule> { None }
    fn compras(&self) -> Option<&dyn ComprasModule> { None }
    fn cxc(&self) -> Option<&dyn CxcModule> { None }
    fn contabilidad(&self) -> Option<&dyn ContabilidadModule> { None }
}
```

El patrón de módulos opcionales es deliberado: ningún ERP tiene todo, y
un provider honesto que solo implementa `facturas` vale más que uno que
promete siete módulos a medias.

### Paso a paso

1. **Crea el crate**: `crates/providers/iskandar-<tu-erp>/` y agrégalo a
   los `members` del `Cargo.toml` del workspace. Usa
   `iskandar-microsip` como referencia de estructura:
   - `src/models.rs` — tu config tipada (la sección
     `[providers.<tu-erp>]` del TOML del usuario). Se deserializa con
     `ProviderConfig::typed::<TuConfig>()`.
   - `src/provider.rs` — la implementación del trait.
   - Si tu integración es FFI a una librería nativa: TODO el `unsafe`
     vive contenido en un solo módulo (`src/dll.rs` o equivalente), con
     comentarios `// SAFETY:` en cada bloque. Hacia arriba solo sale
     Rust seguro.
2. **Implementa `probar_conexion()`** primero. Es lo que corre
   `iskandar test --provider <tu-erp>` y la forma más rápida de validar
   que tu integración respira.
3. **Implementa los módulos que tu ERP soporte de verdad.** Los modelos
   universales viven en `iskandar-core/src/models/` — si tu ERP tiene
   campos que no mapean, van en el campo `extra` de cada documento, no
   en un fork del modelo.
4. **Regístralo en el CLI** (`crates/iskandar-cli/src/main.rs`,
   función `build_registry`). Si tu provider es específico de una
   plataforma, usa `#[cfg(...)]` como hace Microsip con Windows.
5. **Test de integración** en `tests/`, marcado `#[ignore]` y
   configurado por variables de entorno (nunca credenciales en el
   código). Mira `iskandar-microsip/tests/integration.rs`.
6. **Documenta lo no obvio**: versión del ERP contra la que probaste,
   rarezas de su API, y cualquier dato que le ahorre una semana al
   siguiente contribuidor.

### Si tu integración es síncrona (DLLs, protocolos viejos)

Los módulos del trait son `async` porque los ERPs cloud lo son. Para
integraciones síncronas: envuelve el trabajo en
`tokio::task::spawn_blocking` y serializa el acceso con un `Mutex` si la
librería nativa no es thread-safe (ver `iskandar-microsip` como patrón
completo).

---

## Convenciones del proyecto

- **Idiomas**: código e infraestructura en inglés
  (`ProviderRegistry`, `ProviderConfig`); sustantivos de dominio en
  español (`Cliente`, `Factura`, `Renglon`, `Poliza`) — es el
  vocabulario real de los ERPs de la región, CFDI o folio no se
  traducen bien. Documentación pública en español.
- **Errores**: `Result<T, E>` siempre. **Sin `unwrap()` ni `expect()`
  en código de producción** (en tests está bien). Los errores nativos
  de tu ERP se reportan como `IskandarError::Provider { code, message }`.
- **Dinero siempre en `rust_decimal::Decimal`, nunca `f64`** — estos
  números terminan en comprobantes fiscales.
- **IDs**: usa `EntidadId` (entero o string), no asumas el tipo de ID
  de tu ERP en los modelos compartidos.
- **Logging**: `tracing`, no `println!`.
- **Formato y lints**: antes de abrir el PR debe pasar
  ```bash
  cargo fmt --all
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```

---

## Flujo de contribución

1. Haz fork y crea una rama descriptiva (`provider-aspel`,
   `fix-folio-vacio`).
2. Commits pequeños y con mensaje claro (español o inglés, consistente).
3. Abre el PR contra `main` explicando **qué** y **por qué**. Si es un
   provider nuevo, incluye contra qué versión del ERP probaste.
4. Las discusiones de diseño grandes empiezan como **issue**, no como
   PR sorpresa de 3,000 líneas — así nadie pierde trabajo.

¿Encontraste un bug? Abre un issue con: versión de Iskandar, ERP y
versión, qué esperabas, qué pasó, y el log relevante (sin credenciales).

¿Quieres proponer un ERP para el roadmap? Abre un issue con el nombre,
país, qué mecanismo de integración tiene (API, DLL, base de datos) y
qué tan extendido está. Conocimiento local vale oro aquí.

---

## Licencia

Iskandar es **AGPL v3**. Al contribuir, aceptas que tu código se
distribuya bajo la misma licencia. Es el trato: si alguien despliega
Iskandar como servicio y lo mejora, comparte la mejora — el mismo trato
que hicieron con nosotros quienes construyeron nuestras herramientas.

---

## Trato entre personas

Sé directo con el código y amable con la gente. Aquí hay desarrolladores
de toda América Latina, con todos los niveles de experiencia; la
pregunta "básica" de hoy es el provider nuevo de mañana. No se toleran
faltas de respeto.

¿Dudas? Abre un issue o escribe a los autores (ver README).
