# Iskandar

> *El mismo fuego, traducido.*

**Integración open source de ERPs para América Latina.**

---

## Por qué existe Iskandar

En este momento, algún desarrollador en América Latina está mirando un ERP sin API, sin webhooks y con documentación que no se actualiza desde 2003. Necesita conectarlo a un dashboard, un agente de IA, un pipeline de automatización — cualquier cosa moderna. Y está solo.

Todos hemos estado ahí.

Este proyecto existe porque el open source nos dio herramientas cuando no podíamos pagar licencias. PostgreSQL cuando SQL Server estaba fuera de alcance. Linux cuando Windows no era opción. Python, Node, Tesseract, Electron — fuego que alguien encendió y nos pasó sin pedirnos nada.

Iskandar es nuestra forma de devolverlo.

---

## Qué hace

Iskandar es un framework en Rust que expone ERPs heredados como APIs REST modernas y limpias. Abstrae los detalles de implementación — DLLs nativas, protocolos propietarios, bases de datos Firebird — detrás de una interfaz universal de provider que cualquier ERP puede implementar.

Se compila a un binario único, sin dependencias: lo descargas y corre junto al ERP. Sin Python, sin runtime, sin virtualenv.

```rust
use iskandar::ERPClient;

let erp = ERPClient::new("microsip", &config)?;

let factura = erp.facturas()?.crear(NuevaFactura {
    cliente_id: 1042,
    renglones: vec![Renglon { articulo_id: 88, unidades: 3.0, precio: 450.00 }],
})?;
```

Una interfaz. Cualquier ERP.

---

## El ecosistema que cubrimos

Iskandar está construido para el paisaje de ERPs que realmente mueve los negocios latinoamericanos:

| País | ERPs |
|---|---|
| México | Microsip, CONTPAQi, Aspel |
| Colombia | Siigo, World Office |
| Perú / Chile | Defontana, Alegra |
| Centroamérica | Microsip |

Si tu ERP está en esta lista — o no está — puedes construir un provider para él.

---

## Arquitectura

```
iskandar/
├── Cargo.toml                      # Workspace
└── crates/
    ├── iskandar-core/              # Trait ERPProvider + ERPClient + modelos universales
    ├── iskandar-api/               # Rutas axum (capa HTTP opcional)
    ├── iskandar-cli/               # Binario `iskandar` (init / serve / test)
    └── providers/
        └── iskandar-microsip/      # Wrapper libloading sobre ApiMicrosip.dll
```

El trait `ERPProvider` define el contrato. Los providers lo implementan. Al core no le importa cómo: DLL nativa, protocolo propietario o Firebird directo, todo el `unsafe` queda contenido dentro del provider.

Cada provider es un crate independiente con sus propias dependencias y su propia plataforma — Microsip necesita una DLL de Windows, Siigo necesitará un cliente REST, y ninguno contamina al resto. Contribuir un provider nuevo es crear un crate nuevo bajo `crates/providers/`, sin tocar nada más.

---

## Licencia

**AGPL v3.**

Si despliegas Iskandar como servicio y lo mejoras, compartes la mejora. Ese es el trato. Es el mismo trato que hicieron con nosotros las personas que construyeron nuestras herramientas.

---

## Cómo contribuir

Lee `CONTRIBUTING.md` antes de abrir un PR.

La contribución más valiosa ahora mismo es **un nuevo provider**. Si tienes acceso a un ERP que no está en la lista, tienes todo lo que necesitas para construir la siguiente pieza.

---

## Estado

`0.1.0-alpha` — Provider de Microsip en desarrollo activo.

No es production-ready. La arquitectura se está definiendo. Este es el momento correcto para influir en el diseño.

---

## Construido por

[DCA Analytics](https://portafoliodca.netlify.app) — Tlalnepantla, Estado de México.

Nombrado en honor a Alejandro de Macedonia — un muchacho de la periferia que redibujó el mapa del mundo conocido con audacia y muy pocos recursos.

Sabemos cómo se siente.
