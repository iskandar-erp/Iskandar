//! Wrapper sobre `ApiMicrosip.dll` — el ÚNICO módulo del provider con
//! `unsafe`. Todo lo que está arriba de este archivo es Rust seguro.
//!
//! Firmas validadas contra `ApiMspBasicaExt.cs` (referencia oficial Microsip):
//!
//! - DLL Win32 con funciones de convención `stdcall`. Usamos
//!   `extern "system"`, que es stdcall en x86 y la convención C en x64,
//!   cubriendo builds de 32 y 64 bits.
//! - Strings como PChar (null-terminated, estilo C).
//! - La mayoría de funciones retornan `ErrCode` (0 = éxito); el detalle
//!   se recupera con `GetLastErrorMessage`.
//! - `NewDB` es excepción: retorna el handle directamente (> 0 = válido),
//!   NO recibe parámetros de salida.
//! - `SetErrorHandling` es `procedure` en Delphi: no retorna nada.
//! - `GetLastErrorMessage` recibe solo el buffer (sin parámetro de longitud).
//! - La DLL es síncrona y guarda estado global interno (documento en
//!   proceso, último error), así que TODO acceso se serializa con un
//!   `Mutex`. Nunca dos hilos dentro de la DLL a la vez.

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Mutex;

use encoding_rs::WINDOWS_1252;
use iskandar_core::{IskandarError, Result};
use libloading::{Library, Symbol};
use rust_decimal::Decimal;

/// Código de error nativo de la API (0 = sin error).
pub type ErrCode = i32;

// --- Firmas validadas contra ApiMspBasicaExt.cs ---

// procedure SetErrorHandling(ExceptionOnError, MessageOnException: Integer); stdcall;
type FnSetErrorHandling = unsafe extern "system" fn(modo: i32, muestra_dialogos: i32);

// function NewDB: Integer; stdcall;  ← sin parámetros, retorna el handle directamente
type FnNewDb = unsafe extern "system" fn() -> i32;

// function DBConnect(DBHandle: Integer; DataBaseName: PChar; UserName, Password: PChar): Integer; stdcall;
type FnDbConnect = unsafe extern "system" fn(
    handle: i32,
    path: *const c_char,
    usuario: *const c_char,
    password: *const c_char,
) -> ErrCode;

// function NewTrn(DBHandle, TrnType: Integer): Integer; stdcall;  ← retorna trn_handle
type FnNewTrn = unsafe extern "system" fn(db_handle: i32, trn_type: i32) -> i32;

// function DBConnected(DBHandle: Integer): Integer; stdcall;  ← 1=conectado, 0=no
type FnDbConnected = unsafe extern "system" fn(handle: i32) -> i32;

// --- Dataset (consultas SQL de solo lectura) ---

// function TrnStart(TrnHandle: Integer): Integer; stdcall;
type FnTrnStart = unsafe extern "system" fn(trn: i32) -> ErrCode;

// function TrnCommit(TrnHandle: Integer): Integer; stdcall;
type FnTrnCommit = unsafe extern "system" fn(trn: i32) -> ErrCode;

// function NewDtst(TrnHandle: Integer): Integer; stdcall;  ← retorna dtst_handle
type FnNewDtst = unsafe extern "system" fn(trn: i32) -> i32;

// function DtstSelQry(DtstHandle: Integer; Query: PChar): Integer; stdcall;
type FnDtstSelQry = unsafe extern "system" fn(dtst: i32, query: *const c_char) -> ErrCode;

// function DtstOpen(DtstHandle: Integer): Integer; stdcall;
type FnDtstOpen = unsafe extern "system" fn(dtst: i32) -> ErrCode;

// function DtstEof(DtstHandle: Integer): Integer; stdcall;  ← 1=fin, 0=hay más
type FnDtstEof = unsafe extern "system" fn(dtst: i32) -> i32;

// function DtstNext(DtstHandle: Integer): Integer; stdcall;
type FnDtstNext = unsafe extern "system" fn(dtst: i32) -> ErrCode;

// function DtstClose(DtstHandle: Integer): Integer; stdcall;
type FnDtstClose = unsafe extern "system" fn(dtst: i32) -> ErrCode;

// function DtstGetFieldAsString(DtstHandle: Integer; FieldName: PChar; FieldValue: PChar): Integer; stdcall;
type FnDtstGetFieldAsString =
    unsafe extern "system" fn(dtst: i32, field: *const c_char, value: *mut c_char) -> ErrCode;

// function DtstGetFieldAsInteger(DtstHandle: Integer; FieldName: PChar; Var FieldValue: Integer): Integer; stdcall;
type FnDtstGetFieldAsInteger =
    unsafe extern "system" fn(dtst: i32, field: *const c_char, value: *mut i32) -> ErrCode;

// function DtstSetParamAsString(DtstHandle: Integer; ParamName: PChar; ParamValue: PChar): Integer; stdcall;
type FnDtstSetParamAsString =
    unsafe extern "system" fn(dtst: i32, name: *const c_char, value: *const c_char) -> ErrCode;

// function DBDisconnect(DBHandle: Integer): Integer; stdcall;
type FnDbDisconnect = unsafe extern "system" fn(handle: i32) -> ErrCode;

// function GetLastErrorMessage(ErrorMessage: PChar): Integer; stdcall;  ← un solo parámetro
type FnGetLastErrorMessage = unsafe extern "system" fn(buffer: *mut c_char) -> ErrCode;

// --- Servicios Ventas (facturación) — ApiMspVentasExt.pas/.cs ---
//
// Módulo separado de la Api básica de arriba, pero exportado por la MISMA
// ApiMicrosip.dll ya cargada. Firmas contrastadas contra
// `Microsip/ApiMicrosip2026/ApiMspVentasExt.pas` y `.cs`, y la descripción
// de cada parámetro contra `Api - Servicios Ventas - Refer.md`.
//
// NOTA sobre SetFormaCobroFactura: el `.pas` y el manual oficial (línea
// 963 de Refer.md) coinciden en UN solo parámetro `(FormaCobroId: Integer)`.
// El `.cs` de referencia trae una firma con un segundo parámetro
// `NumCtaPago` no documentado en ningún otro lado — se asume que es una
// versión más nueva de la dll que la instalada aquí, y se usa la firma de
// 1 parámetro (.pas + manual, ambos coincidentes) para no arriesgar un
// mismatch de stack en `stdcall` contra la dll real instalada.

// procedure veSetErrorHandling(ExceptionOnError, MessageOnException: Integer); stdcall;
// Módulo de Ventas tiene su PROPIO par de funciones de error, separado de
// SetErrorHandling/GetLastErrorMessage de la Api básica.
type FnVeSetErrorHandling = unsafe extern "system" fn(modo: i32, muestra_dialogos: i32);

// function veGetLastErrorMessage(ErrorMessage: PAnsiChar): Integer; stdcall;
// Regresa el código Y llena el mensaje (no hace falta veGetLastErrorCode
// aparte: el código de la falla ya lo tenemos del rc de la llamada que
// falló, igual que en check() de la Api básica).
type FnVeGetLastErrorMessage = unsafe extern "system" fn(buffer: *mut c_char) -> ErrCode;

// function SetDBVentas(DBHandle: Integer): Integer; stdcall;
type FnSetDbVentas = unsafe extern "system" fn(db_handle: i32) -> ErrCode;

// function ChecaCompatibilidadVentas(HdbMetadatos: Integer): Integer; stdcall;
type FnChecaCompatibilidadVentas = unsafe extern "system" fn(hdb_metadatos: i32) -> ErrCode;

// procedure SetReglasVentas(ExistenciasNegativas, PrecioMinimo: Integer); stdcall;
type FnSetReglasVentas = unsafe extern "system" fn(existencias_negativas: i32, precio_minimo: i32);

// function NuevaFactura(Fecha, Folio: PAnsiChar; ClienteId, DirConsigId, AlmacenId: Integer;
//   TipoDscto: PAnsiChar; Descuento: Double; OrdenCompra, Descripcion: PAnsiChar;
//   Fletes, OtrosCargos, PctjeComis: Double;
//   CondPagoId, VendedorId, ImptoSustituidoId, ImptoSustitutoId: Integer;
//   ImporteCobro: Double; DescripcionCobro: PAnsiChar; PLHandle: Integer): Integer; stdcall;
type FnNuevaFactura = unsafe extern "system" fn(
    fecha: *const c_char,
    folio: *const c_char,
    cliente_id: i32,
    dir_consig_id: i32,
    almacen_id: i32,
    tipo_dscto: *const c_char,
    descuento: f64,
    orden_compra: *const c_char,
    descripcion: *const c_char,
    fletes: f64,
    otros_cargos: f64,
    pctje_comis: f64,
    cond_pago_id: i32,
    vendedor_id: i32,
    impto_sustituido_id: i32,
    impto_sustituto_id: i32,
    importe_cobro: f64,
    descripcion_cobro: *const c_char,
    pl_handle: i32,
) -> ErrCode;

// function DirClienteFactura(DirCliId: Integer): Integer; stdcall;
type FnDirClienteFactura = unsafe extern "system" fn(dir_cli_id: i32) -> ErrCode;

// function SetFormaCobroFactura(FormaCobroId: Integer): Integer; stdcall;  (ver nota arriba)
type FnSetFormaCobroFactura = unsafe extern "system" fn(forma_cobro_id: i32) -> ErrCode;

// function RenglonFactura(ArticuloId: Integer; Unidades, PrecioUnitario,
//   PctjeDscto: Double; Notas: PAnsiChar): Integer; stdcall;
type FnRenglonFactura = unsafe extern "system" fn(
    articulo_id: i32,
    unidades: f64,
    precio_unitario: f64,
    pctje_dscto: f64,
    notas: *const c_char,
) -> ErrCode;

// function AplicaFactura: Integer; stdcall;
type FnAplicaFactura = unsafe extern "system" fn() -> ErrCode;

// procedure AbortaDoctoVentas; stdcall;  ← sin parámetros, sin retorno;
// segura de llamar incluso sin documento en proceso (la doc dice "no se
// hace nada"; "cualquier error es ignorado").
type FnAbortaDoctoVentas = unsafe extern "system" fn();

// function GetDoctoVeId(var DoctoId: Integer): Integer; stdcall;
// IMPORTANTE (Api - Servicios Ventas - Refer.md, sección GetDoctoVeId):
// solo es válido llamarla mientras el documento está "en proceso", es
// decir, ENTRE NuevaFactura y AplicaFactura — nunca después de aplicar.
type FnGetDoctoVeId = unsafe extern "system" fn(docto_id: *mut i32) -> ErrCode;

// --- Objeto lista de parámetros (Api básica) — ApiMspBasicaExt.pas ---
// Confirmado que NO existe FreePL/DisposePL/PLFree en ApiMspBasicaExt.pas:
// por eso el handle se crea una sola vez por proceso y se reutiliza
// (PLClear + PLSetParamValue) en cada factura, ver `MicrosipState::pl_handle`.

// function NewPL: Integer; stdcall;  ← retorna el handle
type FnNewPl = unsafe extern "system" fn() -> i32;

// function PLSetParamValue(PLHandle: Integer; ParamName, ParamValue: PAnsiChar): Integer; stdcall;
type FnPlSetParamValue =
    unsafe extern "system" fn(pl_handle: i32, name: *const c_char, value: *const c_char) -> ErrCode;

// function PLClear(PLHandle: Integer): Integer; stdcall;
type FnPlClear = unsafe extern "system" fn(pl_handle: i32) -> ErrCode;

// --- Servicios Cxc (créditos y cobranza) — ApiMspCxcExt.pas/.cs ---
//
// Módulo separado de Ventas y de la Api básica, pero exportado por la
// MISMA ApiMicrosip.dll ya cargada. Firmas contrastadas contra
// `Microsip/ApiMicrosip2026/ApiMspCxcExt.pas` y `.cs`, y la descripción
// de cada parámetro contra `Api - Servicios Cxc - Refer.md`.

// procedure ccSetErrorHandling(ExceptionOnError, MessageOnException: Integer); stdcall;
// Módulo de Cxc tiene su PROPIO tercer par de funciones de error, separado
// del de Ventas y del de la Api básica.
type FnCcSetErrorHandling = unsafe extern "system" fn(modo: i32, muestra_dialogos: i32);

// function ccGetLastErrorMessage(ErrorMessage: PAnsiChar): Integer; stdcall;
// `ccGetLastErrorCode` también existe en la dll pero no se necesita: el
// código de la falla ya viene del rc de la función que falló (mismo
// razonamiento que con `veGetLastErrorMessage`/`GetLastErrorMessage`).
type FnCcGetLastErrorMessage = unsafe extern "system" fn(buffer: *mut c_char) -> ErrCode;

// function SetDBCxc(DBHandle: Integer): Integer; stdcall;
type FnSetDbCxc = unsafe extern "system" fn(db_handle: i32) -> ErrCode;

// function SetDBMetadatos(DBHandle: Integer): Integer; stdcall;
// Refer.md L189-191: "solo es necesario proporcionar [esta conexión]...
// si se trata de un crédito con concepto de tipo Pago" — pero la doc NO
// dice que llamarla de más (para créditos que no son de tipo Pago) sea
// dañino. Decisión aprobada por el supervisor: se llama SIEMPRE, sin
// intentar detectar el tipo de concepto (ver `MicrosipDll::crear_credito`).
type FnSetDbMetadatos = unsafe extern "system" fn(db_handle: i32) -> ErrCode;

// function ChecaCompatibilidadCxc(HdbMetadatos: Integer): Integer; stdcall;
type FnChecaCompatibilidadCxc = unsafe extern "system" fn(hdb_metadatos: i32) -> ErrCode;

// function NuevoCreditoCc(ConceptoCcId: Integer; Fecha, Folio: PAnsiChar;
//   ClienteId: Integer; Descripcion: PAnsiChar; CobradorId: Integer;
//   PLHandle: Integer): Integer; stdcall;
type FnNuevoCreditoCc = unsafe extern "system" fn(
    concepto_cc_id: i32,
    fecha: *const c_char,
    folio: *const c_char,
    cliente_id: i32,
    descripcion: *const c_char,
    cobrador_id: i32,
    pl_handle: i32,
) -> ErrCode;

// function RenglonCreditoCc(TipoImporte: PAnsiChar; CargoId: Integer;
//   FolioCargo: PAnsiChar; Importe, Impuesto, IvaRetenido, IsrRetenido,
//   DsctoPpag: Double): Integer; stdcall;
//
// Impuesto/IvaRetenido/IsrRetenido: marcados "descontinuados" en la doc
// (Refer.md L567, L572-573: "ya no tendrán efecto alguno") — siempre se
// pasan en 0.0.
//
// TipoImporte fijo en 'R' (Crédito con importe explícito) en v1 —
// decisión confirmada con el supervisor: de los 4 valores documentados
// (Refer.md L550-562) — 'R' (importe explícito), 'A' (saldo por
// acreditar, solo conceptos 'Pago'), 'L' (liquidar por antigüedad), 'S'
// (liquidar saldo completo, importe automático) — solo 'R' encaja sin
// ambigüedad con `AplicacionCargo.importe: Decimal` (no-opcional) del
// modelo universal. 'A'/'L'/'S' quedan FUERA DE ALCANCE v1 (no se expone
// ni siquiera el plumbing de override vía `extra`: abrirían una rama de
// semántica distinta de importe/impuestos no validable contra prod).
//
// RenglonCreditoImpuestoCc (desglose de impuestos por cargo) NO se
// implementa en v1 — confirmado seguro por la doc (Refer.md L639-641,
// L671-674, error 16 en L699): si nunca se invoca, Microsip aplica su
// propio desglose automático a partir de los impuestos reales del cargo
// en BD, sin producir error. Limitación conocida: créditos con desglose
// de impuestos personalizado fuera de alcance.
type FnRenglonCreditoCc = unsafe extern "system" fn(
    tipo_importe: *const c_char,
    cargo_id: i32,
    folio_cargo: *const c_char,
    importe: f64,
    impuesto: f64,
    iva_retenido: f64,
    isr_retenido: f64,
    dscto_ppag: f64,
) -> ErrCode;

// function AplicaCreditoCc: Integer; stdcall;
type FnAplicaCreditoCc = unsafe extern "system" fn() -> ErrCode;

// procedure AbortaDoctoCxc; stdcall;  ← sin parámetros, sin retorno;
// misma semántica que AbortaDoctoVentas: segura de llamar incluso sin
// documento en proceso, cualquier error se ignora.
type FnAbortaDoctoCxc = unsafe extern "system" fn();

// function GetDoctoCCId(var DoctoId: Integer): Integer; stdcall;
// IMPORTANTE (Refer.md L245-253): "Este método debe ser llamado cuando
// haya un crédito en proceso... Un crédito está en proceso desde que se
// invoca 'NuevoCreditoCc' hasta que se invoca AplicaCreditoCc." — mismo
// patrón que GetDoctoVeId: llamar ANTES de AplicaCreditoCc, nunca después.
type FnGetDoctoCcId = unsafe extern "system" fn(docto_id: *mut i32) -> ErrCode;

// NuevoAnticipoCc/AplicaAnticipoCc (anticipos) NO se implementan: firma
// completamente distinta a NuevoCreditoCc (Referencia, CondPagoId,
// Importe directo, ImpuestoId) y ningún campo del modelo universal
// `NuevoCredito`/`AplicacionCargo` mapea a ellos. Fuera de alcance,
// confirmado con el supervisor.

/// Handles de conexión: db_handle para operaciones de BD,
/// trn_handle para el objeto de transacción asignado.
#[derive(Debug, Clone, Copy)]
pub struct DbHandle {
    pub(crate) db: i32,
    pub(crate) trn: i32,
}

/// Lector de campos para una fila activa de un Dataset abierto.
/// Solo válido dentro del closure pasado a [`MicrosipDll::query`].
pub(crate) struct RowReader {
    dtst: i32,
    get_str: FnDtstGetFieldAsString,
    get_int: FnDtstGetFieldAsInteger,
}

impl RowReader {
    /// Lee un campo de texto. Falla si el campo no existe.
    pub fn str_field(&self, name: &str) -> iskandar_core::Result<String> {
        let c_name = cstring(name)?;
        let mut buf = vec![0u8; 2048];
        // SAFETY: buf de 2048 bytes válido durante la llamada.
        let rc = unsafe {
            (self.get_str)(self.dtst, c_name.as_ptr(), buf.as_mut_ptr() as *mut c_char)
        };
        if rc != 0 {
            return Err(IskandarError::Provider {
                code: rc,
                message: format!("error leyendo campo '{name}'"),
            });
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        // La DLL puede devolver Windows-1252 en mensajes de error;
        // los datos de usuario suelen ser ASCII/UTF-8.
        Ok(String::from_utf8_lossy(&buf[..end]).trim().to_owned())
    }

    /// Lee un campo de texto; retorna `None` si está vacío o hay error.
    pub fn opt_str(&self, name: &str) -> Option<String> {
        self.str_field(name)
            .ok()
            .and_then(|s| if s.is_empty() { None } else { Some(s) })
    }

    /// Lee un campo entero.
    pub fn int_field(&self, name: &str) -> iskandar_core::Result<i32> {
        let c_name = cstring(name)?;
        let mut val: i32 = 0;
        // SAFETY: val es válido durante la llamada.
        let rc = unsafe { (self.get_int)(self.dtst, c_name.as_ptr(), &mut val) };
        if rc != 0 {
            return Err(IskandarError::Provider {
                code: rc,
                message: format!("error leyendo campo int '{name}'"),
            });
        }
        Ok(val)
    }

    /// Lee un campo monetario/decimal almacenado sin punto (p. ej. "150000"
    /// con `escala=2` → 1500.00), como usan los campos BIGINT de Microsip
    /// (montos en DOCTOS_VE, PRECIO en PRECIOS_ARTICULOS, etc.).
    ///
    /// Retorna `None` si el campo está vacío/ausente (p. ej. lado derecho
    /// de un LEFT JOIN sin fila) o si el valor no es un entero parseable —
    /// nunca produce `0` como sustituto de "sin dato".
    pub fn opt_dec(&self, name: &str, escala: u32) -> Option<Decimal> {
        self.opt_str(name)
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|entero| Decimal::new(entero, escala))
    }
}

/// Estado compartido de los módulos de Ventas y Cxc que persiste entre
/// llamadas a [`MicrosipDll::crear_factura`]/[`MicrosipDll::crear_credito`].
/// Vive DENTRO del mismo `Mutex` que ya serializa todo acceso a la dll —
/// no necesita sincronización propia.
///
/// Los flags de compatibilidad son SEPARADOS por módulo a propósito
/// (decisión confirmada con el supervisor): `ChecaCompatibilidadVentas` y
/// `ChecaCompatibilidadCxc` son chequeos distintos que gatean módulos
/// distintos — un solo flag compartido saltaría el chequeo de uno de los
/// dos si el otro ya corrió. `pl_handle`, en cambio, SÍ se comparte entre
/// ambos módulos: es un objeto de la Api básica (`NewPL`/`PLSetParamValue`
/// /`PLClear` en `ApiMspBasicaExt.pas`), no específico de Ventas ni de
/// Cxc, y el `Mutex` serializa todo acceso a la dll así que no hay riesgo
/// de uso concurrente entre módulos; `PLClear` antes de cada uso evita
/// contaminación cruzada de parámetros entre una factura y un crédito.
#[derive(Default)]
struct MicrosipState {
    /// `true` una vez que `ChecaCompatibilidadVentas` corrió con éxito.
    /// Según el ejemplo de inicialización oficial ("Pasos para registrar
    /// facturas", Api - Servicios Ventas - Refer.md), este chequeo (que
    /// requiere abrir una conexión aparte a Metadatos.fdb) es parte de la
    /// inicialización DEL PROGRAMA, no de cada factura — se hace una sola
    /// vez por proceso. `veSetErrorHandling`/`SetReglasVentas` en cambio
    /// se reinvocan en cada llamada porque son baratos e idempotentes.
    compatibilidad_ventas_checada: bool,
    /// Igual que `compatibilidad_ventas_checada` pero para
    /// `ChecaCompatibilidadCxc` (módulo Cxc, gated una sola vez por
    /// proceso independientemente del flag de Ventas).
    compatibilidad_cxc_checada: bool,
    /// Handle de la lista de parámetros, creado una sola vez con `NewPL`
    /// y reutilizado (`PLClear` + `PLSetParamValue`) en cada factura o
    /// crédito — la Api básica no expone `FreePL`/`DisposePL`
    /// (confirmado: no aparece en `ApiMspBasicaExt.pas`), así que crear
    /// un handle nuevo por operación sería una fuga de memoria del
    /// proceso de la dll que dura hasta cerrar la aplicación.
    pl_handle: Option<i32>,
}

/// Parámetros de un renglón, ya resueltos a lo que espera `RenglonFactura`
/// (defaults de Microsip aplicados por el llamador en `provider.rs`).
pub struct RenglonParams {
    pub articulo_id: i32,
    pub unidades: f64,
    /// -1.0 = precio según políticas Microsip.
    pub precio_unitario: f64,
    /// -1.0 = descuento según políticas Microsip.
    pub pctje_dscto: f64,
    /// Texto libre; se transcodifica a Windows-1252 (PAnsiChar) igual que
    /// el resto de los campos de texto de este módulo.
    pub notas: String,
}

/// Parámetros de una factura, ya resueltos a lo que espera `NuevaFactura`
/// (defaults de Microsip aplicados por el llamador en `provider.rs` — este
/// módulo solo hace la llamada FFI, no decide defaults de negocio).
pub struct FacturaParams {
    /// Ya formateada "D/M/A" (o "D/M/A HH:NN").
    pub fecha: String,
    /// "" = folio automático "sin serie".
    pub folio: String,
    pub cliente_id: i32,
    /// 0 = dirección principal del cliente.
    pub dir_consig_id: i32,
    /// 0 = almacén principal de la empresa.
    pub almacen_id: i32,
    /// "P" (porcentaje) o "I" (importe).
    pub tipo_dscto: String,
    pub descuento: f64,
    pub orden_compra: String,
    pub descripcion: String,
    pub fletes: f64,
    pub otros_cargos: f64,
    /// -1.0 = según políticas de comisión de vendedores.
    pub pctje_comis: f64,
    /// 0 = condición de pago del cliente.
    pub cond_pago_id: i32,
    /// 0 = vendedor del cliente.
    pub vendedor_id: i32,
    /// 0 = sin sustitución de impuesto (debe ir junto con impto_sustituto_id).
    pub impto_sustituido_id: i32,
    pub impto_sustituto_id: i32,
    /// -1.0 = cobro automático del total. SIN default seguro documentado
    /// para "no cobrar" — `provider.rs` exige que el llamador lo
    /// especifique explícitamente en `extra`, nunca se infiere aquí.
    pub importe_cobro: f64,
    pub descripcion_cobro: String,
    /// Pares (nombre, valor) para `PLHandle`. Vacío = se pasa `-1` a
    /// `NuevaFactura` (documentado: "no hay lista de parámetros, se
    /// toma el default de cada uno").
    pub lista_parametros: Vec<(String, String)>,
    /// Si viene, se llama `DirClienteFactura` después de `NuevaFactura`.
    pub dir_cliente_id: Option<i32>,
    /// Se llama `SetFormaCobroFactura` una vez por cada elemento (solo
    /// aplica a modalidad CFD/CBB/CFDI 3.2, ver doc de la función).
    pub formas_cobro_cbb: Vec<i32>,
    pub renglones: Vec<RenglonParams>,
}

/// Aplicación de un crédito contra un cargo, ya resuelta a lo que espera
/// `RenglonCreditoCc` (defaults de negocio aplicados por el llamador en
/// `cxc.rs`, igual que `RenglonParams`/`FacturaParams` para Ventas).
pub struct AplicacionCreditoParams {
    /// 0 si el cargo se identifica solo por `folio_cargo`. SUPUESTO NO
    /// VERIFICADO: la doc no aclara explícitamente si `CargoId = 0` +
    /// `FolioCargo` no vacío es suficiente para localizar el cargo — se
    /// asume que sí porque `AplicacionCargo.cargo_id` del modelo
    /// universal es `Option<EntidadId>`. Documentar como riesgo si algún
    /// día se valida en vivo.
    pub cargo_id: i32,
    pub folio_cargo: String,
    pub importe: f64,
    /// -1.0 = "determinarlo automáticamente" (Refer.md L576). ADVERTENCIA
    /// (edge conocido, no auto-detectado): si la preferencia "Emitir CFDI
    /// de los cobros" de la empresa está activa, la doc exige que este
    /// valor sea EXACTAMENTE 0.0 (Refer.md L579-580) — con -1.0 un cobro
    /// fiscal podría fallar. Este código NO detecta esa preferencia;
    /// permite override a 0.0 vía `extra` (ver `cxc.rs`).
    pub dscto_ppag: f64,
}

/// Parámetros de un crédito, ya resueltos a lo que espera `NuevoCreditoCc`
/// y `RenglonCreditoCc` (defaults de negocio aplicados por el llamador
/// en `cxc.rs` — este módulo solo hace las llamadas FFI, no decide
/// defaults).
pub struct CreditoParams {
    /// 0 = concepto "Cobro" (Refer.md L295-296).
    pub concepto_cc_id: i32,
    /// Ya formateada "D/M/A" (o "D/M/A HH:NN").
    pub fecha: String,
    /// "" = folio automático.
    pub folio: String,
    pub cliente_id: i32,
    pub descripcion: String,
    /// 0 = cobrador asignado al cliente (Refer.md L312-313; "si el
    /// concepto del crédito no integra comisiones, este dato no importa").
    pub cobrador_id: i32,
    /// Pares (nombre, valor) para `PLHandle`. Vacío = se pasa `-1` a
    /// `NuevoCreditoCc` (mismo sentinela documentado que en `NuevaFactura`).
    pub lista_parametros: Vec<(String, String)>,
    /// Un `RenglonCreditoCc` por cada elemento, TipoImporte fijo en 'R'
    /// (ver nota junto a `FnRenglonCreditoCc`).
    pub aplicaciones: Vec<AplicacionCreditoParams>,
}

pub struct MicrosipDll {
    lib: Library,
    /// La DLL no es thread-safe: un solo hilo dentro a la vez. También
    /// guarda el estado del módulo de Ventas que debe persistir entre
    /// llamadas (ver [`MicrosipState`]).
    lock: Mutex<MicrosipState>,
}

impl MicrosipDll {
    /// Carga la DLL desde disco. No abre ninguna conexión todavía.
    pub fn load(path: &str) -> Result<Self> {
        // SAFETY: cargar una librería ejecuta su DllMain. Confiamos en
        // que la ruta apunta a la ApiMicrosip.dll legítima — es
        // responsabilidad de la configuración del usuario.
        let lib = unsafe { Library::new(path) }.map_err(|e| {
            IskandarError::Connection(format!("no se pudo cargar la DLL '{path}': {e}"))
        })?;
        Ok(Self {
            lib,
            lock: Mutex::new(MicrosipState::default()),
        })
    }

    /// Abre una conexión a la base de empresa y regresa su handle.
    ///
    /// Flujo según referencia oficial:
    /// `SetErrorHandling(0,0)` → `NewDB()` → `DBConnect(handle, path, user, pass)`
    pub fn connect(&self, db_path: &str, usuario: &str, password: &str) -> Result<DbHandle> {
        let _guard = self.guard()?;
        self.connect_internal(db_path, usuario, password)
    }

    /// Igual que [`Self::connect`] pero SIN adquirir el `Mutex` — para
    /// usarse desde código que ya sostiene el guard a nivel superior (p.
    /// ej. la conexión aparte a Metadatos dentro de
    /// [`Self::crear_factura`]). Llamar esto sin el guard ya tomado deja
    /// de estar protegido contra acceso concurrente a la dll.
    fn connect_internal(&self, db_path: &str, usuario: &str, password: &str) -> Result<DbHandle> {
        let set_error_handling: Symbol<FnSetErrorHandling> = self.symbol(b"SetErrorHandling\0")?;
        let new_db: Symbol<FnNewDb> = self.symbol(b"NewDB\0")?;
        let db_connect: Symbol<FnDbConnect> = self.symbol(b"DBConnect\0")?;

        let c_path = cstring(db_path)?;
        let c_user = cstring(usuario)?;
        let c_pass = cstring(password)?;

        let new_trn: Symbol<FnNewTrn> = self.symbol(b"NewTrn\0")?;

        // SAFETY: firmas validadas contra ApiMspBasicaExt.cs oficial.
        // Los CStrings viven durante todo el bloque unsafe.
        // El objeto DB de IBX requiere un TIBTransaction asignado ANTES
        // de llamar DBConnect; por eso NewTrn va entre NewDB y DBConnect.
        let (db, trn) = unsafe {
            tracing::debug!("SetErrorHandling(0, 0)");
            set_error_handling(0, 0);

            tracing::debug!("NewDB()");
            let db_h = new_db();
            if db_h <= 0 {
                let msg = self.last_error_message().unwrap_or_else(|| "(sin mensaje)".into());
                return Err(IskandarError::Connection(format!(
                    "NewDB() falló (handle={db_h}): {msg}"
                )));
            }

            tracing::debug!("NewTrn(db={db_h}, type=0)");
            let trn_h = new_trn(db_h, 0);
            if trn_h <= 0 {
                let msg = self.last_error_message().unwrap_or_else(|| "(sin mensaje)".into());
                return Err(IskandarError::Connection(format!(
                    "NewTrn() falló (handle={trn_h}): {msg}"
                )));
            }

            tracing::debug!("DBConnect(db={db_h}, path={db_path:?})");
            self.check(db_connect(db_h, c_path.as_ptr(), c_user.as_ptr(), c_pass.as_ptr()))?;
            (db_h, trn_h)
        };

        tracing::debug!("Conexión establecida, db={db} trn={trn}");
        Ok(DbHandle { db, trn })
    }

    /// Verifica que el handle sigue activo. Útil en `probar_conexion`.
    pub fn connected(&self, handle: DbHandle) -> Result<bool> {
        let _guard = self.guard()?;
        let db_connected: Symbol<FnDbConnected> = self.symbol(b"DBConnected\0")?;
        // SAFETY: firma según ApiMspBasicaExt.cs.
        let rc = unsafe { db_connected(handle.db) };
        Ok(rc == 1)
    }

    pub fn disconnect(&self, handle: DbHandle) -> Result<()> {
        let _guard = self.guard()?;
        self.disconnect_internal(handle)
    }

    /// Igual que [`Self::disconnect`] pero sin adquirir el `Mutex` — ver
    /// [`Self::connect_internal`].
    fn disconnect_internal(&self, handle: DbHandle) -> Result<()> {
        let db_disconnect: Symbol<FnDbDisconnect> = self.symbol(b"DBDisconnect\0")?;
        tracing::debug!("DBDisconnect(db={})", handle.db);
        // SAFETY: handle obtenido de connect()/connect_internal(); firma
        // según referencia.
        let rc = unsafe { db_disconnect(handle.db) };
        self.check(rc)
    }

    /// Ejecuta una query SQL de solo lectura y mapea cada fila con `map_row`.
    ///
    /// Gestiona el ciclo completo: `TrnStart` → `NewDtst` → `DtstSelQry` →
    /// `DtstOpen` → iteración → `DtstClose` → `TrnCommit`.
    /// Ejecuta una query SQL de solo lectura y mapea cada fila con `map_row`.
    ///
    /// `params` son pares `(nombre, valor)` que se enlazan como parámetros
    /// con nombre (`:nombre` en la SQL) vía `DtstSetParamAsString` — nunca
    /// se interpolan directamente, por lo que no hay riesgo de SQL injection.
    ///
    /// Gestiona el ciclo: `TrnStart` → `NewDtst` → `DtstSelQry` →
    /// bind params → `DtstOpen` → iteración → `DtstClose` → `TrnCommit`.
    pub(crate) fn query<T, F>(
        &self,
        handle: DbHandle,
        sql: &str,
        params: &[(&str, &str)],
        mut map_row: F,
    ) -> Result<Vec<T>>
    where
        F: FnMut(&RowReader) -> Result<T>,
    {
        let _guard = self.guard()?;

        let trn_start: Symbol<FnTrnStart> = self.symbol(b"TrnStart\0")?;
        let trn_commit: Symbol<FnTrnCommit> = self.symbol(b"TrnCommit\0")?;
        let new_dtst: Symbol<FnNewDtst> = self.symbol(b"NewDtst\0")?;
        let dtst_sel_qry: Symbol<FnDtstSelQry> = self.symbol(b"DtstSelQry\0")?;
        let dtst_set_param: Symbol<FnDtstSetParamAsString> =
            self.symbol(b"DtstSetParamAsString\0")?;
        let dtst_open: Symbol<FnDtstOpen> = self.symbol(b"DtstOpen\0")?;
        let dtst_eof: Symbol<FnDtstEof> = self.symbol(b"DtstEof\0")?;
        let dtst_next: Symbol<FnDtstNext> = self.symbol(b"DtstNext\0")?;
        let dtst_close: Symbol<FnDtstClose> = self.symbol(b"DtstClose\0")?;
        let dtst_get_str: Symbol<FnDtstGetFieldAsString> =
            self.symbol(b"DtstGetFieldAsString\0")?;
        let dtst_get_int: Symbol<FnDtstGetFieldAsInteger> =
            self.symbol(b"DtstGetFieldAsInteger\0")?;

        let c_sql = cstring(sql)?;
        // Preparar CStrings de parámetros antes del bloque unsafe.
        let c_params: Vec<(CString, CString)> = params
            .iter()
            .map(|(n, v)| Ok((cstring(n)?, cstring(v)?)))
            .collect::<Result<_>>()?;

        // SAFETY: todas las firmas validadas contra ApiMspBasicaExt.cs.
        unsafe {
            self.check(trn_start(handle.trn))?;

            let dtst = new_dtst(handle.trn);
            if dtst <= 0 {
                let msg = self.last_error_message().unwrap_or_else(|| "(sin mensaje)".into());
                return Err(IskandarError::Connection(format!("NewDtst() falló: {msg}")));
            }

            self.check(dtst_sel_qry(dtst, c_sql.as_ptr()))?;

            for (c_name, c_val) in &c_params {
                self.check(dtst_set_param(dtst, c_name.as_ptr(), c_val.as_ptr()))?;
            }

            self.check(dtst_open(dtst))?;

            let reader = RowReader {
                dtst,
                get_str: *dtst_get_str,
                get_int: *dtst_get_int,
            };

            let mut rows = Vec::new();
            while dtst_eof(dtst) == 0 {
                rows.push(map_row(&reader)?);
                self.check(dtst_next(dtst))?;
            }

            dtst_close(dtst);
            self.check(trn_commit(handle.trn))?;
            Ok(rows)
        }
    }

    /// Ejecuta el flujo completo de alta de una factura bajo UN SOLO
    /// guard del `Mutex`: conecta a la empresa, inicializa el módulo de
    /// Ventas (una sola vez por proceso lo que requiere Metadatos, cada
    /// vez lo demás), registra encabezado + renglones, aplica, y regresa
    /// el `DOCTO_VE_ID` real.
    ///
    /// Cualquier falla en cualquier paso desde `NuevaFactura` en adelante
    /// dispara `AbortaDoctoVentas` antes de propagar el error original —
    /// el error del aborto (si lo hay) nunca reemplaza al original.
    ///
    /// `metadatos_path`: si es `None`, se omite `ChecaCompatibilidadVentas`
    /// (la doc lo marca como recomendado, no como requisito duro para dar
    /// de alta documentos) con un `tracing::warn!`.
    pub fn crear_factura(
        &self,
        db_path: &str,
        usuario: &str,
        password: &str,
        metadatos_path: Option<&str>,
        reglas_ventas: (i32, i32),
        params: FacturaParams,
    ) -> Result<i32> {
        let mut guard = self.guard()?;

        let ve_set_error_handling: Symbol<FnVeSetErrorHandling> =
            self.symbol(b"veSetErrorHandling\0")?;
        let set_db_ventas: Symbol<FnSetDbVentas> = self.symbol(b"SetDBVentas\0")?;
        let checa_compat: Symbol<FnChecaCompatibilidadVentas> =
            self.symbol(b"ChecaCompatibilidadVentas\0")?;
        let set_reglas_ventas: Symbol<FnSetReglasVentas> = self.symbol(b"SetReglasVentas\0")?;
        let nueva_factura: Symbol<FnNuevaFactura> = self.symbol(b"NuevaFactura\0")?;
        let dir_cliente_factura: Symbol<FnDirClienteFactura> =
            self.symbol(b"DirClienteFactura\0")?;
        let set_forma_cobro: Symbol<FnSetFormaCobroFactura> =
            self.symbol(b"SetFormaCobroFactura\0")?;
        let renglon_factura: Symbol<FnRenglonFactura> = self.symbol(b"RenglonFactura\0")?;
        let aplica_factura: Symbol<FnAplicaFactura> = self.symbol(b"AplicaFactura\0")?;
        let aborta_doc: Symbol<FnAbortaDoctoVentas> = self.symbol(b"AbortaDoctoVentas\0")?;
        let get_docto_id: Symbol<FnGetDoctoVeId> = self.symbol(b"GetDoctoVeId\0")?;
        let new_pl: Symbol<FnNewPl> = self.symbol(b"NewPL\0")?;
        let pl_set_param: Symbol<FnPlSetParamValue> = self.symbol(b"PLSetParamValue\0")?;
        let pl_clear: Symbol<FnPlClear> = self.symbol(b"PLClear\0")?;

        // Conexión a la empresa: una nueva conexión por-llamada, mismo
        // patrón que connect()/query() ya usan para lecturas.
        let empresa = self.connect_internal(db_path, usuario, password)?;

        let resultado: Result<i32> = (|| -> Result<i32> {
            // SAFETY: todas las firmas contrastadas contra
            // ApiMspVentasExt.pas/.cs y ApiMspBasicaExt.pas (ver arriba).
            // Los CStrings viven durante todo este bloque.
            unsafe {
                tracing::debug!("veSetErrorHandling(0, 0)");
                ve_set_error_handling(0, 0);

                tracing::debug!("SetDBVentas(db={})", empresa.db);
                self.check_ventas(set_db_ventas(empresa.db))?;

                if !guard.compatibilidad_ventas_checada {
                    match metadatos_path {
                        Some(meta_path) => {
                            tracing::debug!("ChecaCompatibilidadVentas vía {meta_path}");
                            let meta = self.connect_internal(meta_path, usuario, password)?;
                            let check_rc = checa_compat(meta.db);
                            // Desconectar Metadatos pase lo que pase — ya
                            // no se necesita más allá de este chequeo.
                            self.disconnect_internal(meta).ok();
                            self.check_ventas(check_rc)?;
                            guard.compatibilidad_ventas_checada = true;
                        }
                        None => {
                            tracing::warn!(
                                "MicrosipConfig.metadatos_path no configurado: se omite \
                                 ChecaCompatibilidadVentas (recomendado por la doc oficial, \
                                 no obligatorio para registrar documentos)"
                            );
                        }
                    }
                }

                tracing::debug!(
                    "SetReglasVentas({}, {})",
                    reglas_ventas.0,
                    reglas_ventas.1
                );
                set_reglas_ventas(reglas_ventas.0, reglas_ventas.1);

                // PLHandle: -1 si no hay parámetros de lista (documentado:
                // "no hay lista de parámetros, se toma el default de cada
                // uno"). Si hay, reutilizamos el ÚNICO handle del proceso.
                let pl_handle = if params.lista_parametros.is_empty() {
                    -1
                } else {
                    let handle = match guard.pl_handle {
                        Some(h) => h,
                        None => {
                            let h = new_pl();
                            if h <= 0 {
                                let msg = self
                                    .ve_last_error_message()
                                    .unwrap_or_else(|| "(sin mensaje)".into());
                                return Err(IskandarError::Connection(format!(
                                    "NewPL() falló (handle={h}): {msg}"
                                )));
                            }
                            guard.pl_handle = Some(h);
                            h
                        }
                    };
                    // Limpiamos valores de una factura anterior antes de
                    // volver a llenar la lista con los de esta factura.
                    self.check_ventas(pl_clear(handle))?;
                    for (nombre, valor) in &params.lista_parametros {
                        let c_nombre = cstring(nombre)?;
                        let c_valor = cstring_ansi(valor)?;
                        self.check_ventas(pl_set_param(
                            handle,
                            c_nombre.as_ptr(),
                            c_valor.as_ptr(),
                        ))?;
                    }
                    handle
                };

                let c_fecha = cstring(&params.fecha)?;
                let c_folio = cstring(&params.folio)?;
                let c_tipo_dscto = cstring(&params.tipo_dscto)?;
                let c_orden_compra = cstring_ansi(&params.orden_compra)?;
                let c_descripcion = cstring_ansi(&params.descripcion)?;
                let c_descripcion_cobro = cstring_ansi(&params.descripcion_cobro)?;

                tracing::debug!(
                    "NuevaFactura(cliente={}, folio={:?})",
                    params.cliente_id,
                    params.folio
                );
                self.check_ventas(nueva_factura(
                    c_fecha.as_ptr(),
                    c_folio.as_ptr(),
                    params.cliente_id,
                    params.dir_consig_id,
                    params.almacen_id,
                    c_tipo_dscto.as_ptr(),
                    params.descuento,
                    c_orden_compra.as_ptr(),
                    c_descripcion.as_ptr(),
                    params.fletes,
                    params.otros_cargos,
                    params.pctje_comis,
                    params.cond_pago_id,
                    params.vendedor_id,
                    params.impto_sustituido_id,
                    params.impto_sustituto_id,
                    params.importe_cobro,
                    c_descripcion_cobro.as_ptr(),
                    pl_handle,
                ))?;

                if let Some(dir_cli_id) = params.dir_cliente_id {
                    tracing::debug!("DirClienteFactura({dir_cli_id})");
                    self.check_ventas(dir_cliente_factura(dir_cli_id))?;
                }

                for forma_cobro_id in &params.formas_cobro_cbb {
                    tracing::debug!("SetFormaCobroFactura({forma_cobro_id})");
                    self.check_ventas(set_forma_cobro(*forma_cobro_id))?;
                }

                for renglon in &params.renglones {
                    let c_notas = cstring_ansi(&renglon.notas)?;
                    tracing::debug!(
                        "RenglonFactura(articulo={}, unidades={})",
                        renglon.articulo_id,
                        renglon.unidades
                    );
                    self.check_ventas(renglon_factura(
                        renglon.articulo_id,
                        renglon.unidades,
                        renglon.precio_unitario,
                        renglon.pctje_dscto,
                        c_notas.as_ptr(),
                    ))?;
                }

                // IMPORTANTE: GetDoctoVeId solo es válido MIENTRAS el
                // documento está "en proceso", es decir, ANTES de
                // AplicaFactura (ver nota en FnGetDoctoVeId arriba). Si se
                // llamara después, la doc indica error "1 = No hay
                // documento en proceso".
                let mut docto_id: i32 = 0;
                self.check_ventas(get_docto_id(&mut docto_id))?;

                tracing::debug!("AplicaFactura()");
                self.check_ventas(aplica_factura())?;

                Ok(docto_id)
            }
        })();

        if resultado.is_err() {
            tracing::warn!(
                "crear_factura falló ({:?}); ejecutando AbortaDoctoVentas para no dejar \
                 un documento a medio registrar",
                resultado.as_ref().err()
            );
            // SAFETY: AbortaDoctoVentas es segura de llamar incluso sin
            // documento en proceso (la doc dice "no se hace nada") y
            // "cualquier error es ignorado" — no puede enmascarar el
            // error original porque no propaga ninguno.
            unsafe { aborta_doc() };
        }

        // Cerramos la conexión de empresa pase lo que pase; ignoramos el
        // error de desconexión para no enmascarar el error original de
        // `resultado` (que ya tiene prioridad).
        self.disconnect_internal(empresa).ok();

        resultado
    }

    /// Ejecuta el flujo completo de alta de un crédito de Cxc bajo UN SOLO
    /// guard del `Mutex`, calcado 1:1 del patrón de [`Self::crear_factura`]:
    /// conecta a la empresa, inicializa el módulo de Cxc (una sola vez por
    /// proceso lo que requiere Metadatos, cada vez lo demás), registra
    /// encabezado + renglones, aplica, y regresa el `DOCTO_CC_ID` real.
    ///
    /// Cualquier falla en cualquier paso desde `NuevoCreditoCc` en
    /// adelante dispara `AbortaDoctoCxc` antes de propagar el error
    /// original — el error del aborto (si lo hay) nunca reemplaza al
    /// original.
    ///
    /// DECISIÓN ARQUITECTÓNICA (aprobada por el supervisor, ver
    /// `Contexto/MEMORIA_AGENTES.md`): la conexión a Metadatos NO es
    /// persistente en el estado del `Mutex` (a diferencia de lo que
    /// sugiere el ejemplo de inicialización oficial del manual de Cxc,
    /// que la abre una sola vez para toda la vida del programa). Aquí se
    /// abre POR-OPERACIÓN, igual que la conexión de Metadatos en
    /// `crear_factura`: introducir un handle de vida-de-proceso en el
    /// Mutex agregaría lógica de reconexión/shutdown que hoy no existe en
    /// el codebase — superficie de bug nueva en un camino de escritura no
    /// validable contra prod.
    ///
    /// `metadatos_path`: si es `None`, se omiten tanto
    /// `ChecaCompatibilidadCxc` como `SetDBMetadatos` (con un
    /// `tracing::warn!`) — un crédito de concepto "Pago" podría fallar en
    /// `AplicaCreditoCc` sin acceso a la info fiscal de la forma de cobro.
    pub fn crear_credito(
        &self,
        db_path: &str,
        usuario: &str,
        password: &str,
        metadatos_path: Option<&str>,
        params: CreditoParams,
    ) -> Result<i32> {
        let mut guard = self.guard()?;

        let cc_set_error_handling: Symbol<FnCcSetErrorHandling> =
            self.symbol(b"ccSetErrorHandling\0")?;
        let set_db_cxc: Symbol<FnSetDbCxc> = self.symbol(b"SetDBCxc\0")?;
        let set_db_metadatos: Symbol<FnSetDbMetadatos> = self.symbol(b"SetDBMetadatos\0")?;
        let checa_compat_cxc: Symbol<FnChecaCompatibilidadCxc> =
            self.symbol(b"ChecaCompatibilidadCxc\0")?;
        let nuevo_credito_cc: Symbol<FnNuevoCreditoCc> = self.symbol(b"NuevoCreditoCc\0")?;
        let renglon_credito_cc: Symbol<FnRenglonCreditoCc> = self.symbol(b"RenglonCreditoCc\0")?;
        let aplica_credito_cc: Symbol<FnAplicaCreditoCc> = self.symbol(b"AplicaCreditoCc\0")?;
        let aborta_docto_cxc: Symbol<FnAbortaDoctoCxc> = self.symbol(b"AbortaDoctoCxc\0")?;
        let get_docto_cc_id: Symbol<FnGetDoctoCcId> = self.symbol(b"GetDoctoCCId\0")?;
        let new_pl: Symbol<FnNewPl> = self.symbol(b"NewPL\0")?;
        let pl_set_param: Symbol<FnPlSetParamValue> = self.symbol(b"PLSetParamValue\0")?;
        let pl_clear: Symbol<FnPlClear> = self.symbol(b"PLClear\0")?;

        // Conexión a la empresa: una nueva conexión por-llamada, mismo
        // patrón que connect()/crear_factura() ya usan.
        let empresa = self.connect_internal(db_path, usuario, password)?;

        // Conexión a Metadatos: se abre UNA vez aquí y sirve para DOS
        // propósitos — `ChecaCompatibilidadCxc` (gated, una vez por
        // proceso) y `SetDBMetadatos` (llamado SIEMPRE, sin gating por
        // tipo de concepto — ver nota en `FnSetDbMetadatos`). Se mantiene
        // abierta hasta después de `AplicaCreditoCc` (créditos de
        // concepto "Pago" necesitan la info fiscal de la forma de cobro
        // durante la aplicación, no solo al inicio) y se cierra al final
        // junto con la conexión de empresa.
        //
        // RIESGO NO VALIDADO CONTRA PROD, específico de créditos con
        // concepto tipo "Pago": `connect_internal` llama `NewTrn(db, 0)`
        // con el tipo de transacción HARDCODEADO en 0 para TODA conexión.
        // El manual oficial de Cxc (Refer.md L918-941) documenta DOS
        // tipos de transacción DISTINTOS para las conexiones de
        // Metadatos: tipo 3 para la conexión transitoria de
        // `ChecaCompatibilidadCxc`, tipo 4 ("DBMetadatosLectura", nota
        // explícita L931 "Se utiliza el tipo de transacción 4") para la
        // conexión de lectura que usa `SetDBMetadatos`. Aquí se reutiliza
        // el patrón tipo-0 existente (el mismo que ya usa, con validación
        // en vivo, `ChecaCompatibilidadVentas` en `crear_factura`) por
        // consistencia y simplicidad, PERO esto es una DESVIACIÓN
        // CONOCIDA del manual, no verificada contra un crédito real de
        // concepto "Pago". Si algún día un crédito "Pago" falla en
        // `AplicaCreditoCc` con un error relacionado a datos fiscales de
        // la forma de cobro, lo PRIMERO a intentar es parametrizar
        // `NewTrn` a tipo 4 para esta conexión de Metadatos.
        let metadatos = match metadatos_path {
            Some(meta_path) => Some(self.connect_internal(meta_path, usuario, password)?),
            None => None,
        };

        let resultado: Result<i32> = (|| -> Result<i32> {
            // SAFETY: todas las firmas contrastadas contra
            // ApiMspCxcExt.pas/.cs y ApiMspBasicaExt.pas (ver arriba). Los
            // CStrings viven durante todo este bloque.
            unsafe {
                tracing::debug!("ccSetErrorHandling(0, 0)");
                cc_set_error_handling(0, 0);

                tracing::debug!("SetDBCxc(db={})", empresa.db);
                self.check_cxc(set_db_cxc(empresa.db))?;

                match metadatos {
                    Some(meta) => {
                        if !guard.compatibilidad_cxc_checada {
                            tracing::debug!("ChecaCompatibilidadCxc(db={})", meta.db);
                            self.check_cxc(checa_compat_cxc(meta.db))?;
                            guard.compatibilidad_cxc_checada = true;
                        }

                        // SetDBMetadatos: llamado SIEMPRE (decisión
                        // aprobada por el supervisor), sin intentar
                        // detectar si el concepto del crédito es "Pago" —
                        // ver nota en `FnSetDbMetadatos`.
                        tracing::debug!("SetDBMetadatos(db={})", meta.db);
                        self.check_cxc(set_db_metadatos(meta.db))?;
                    }
                    None => {
                        tracing::warn!(
                            "MicrosipConfig.metadatos_path no configurado: se omiten \
                             ChecaCompatibilidadCxc y SetDBMetadatos — un crédito de \
                             concepto 'Pago' podría fallar en AplicaCreditoCc sin acceso \
                             a la info fiscal de la forma de cobro"
                        );
                    }
                }

                // PLHandle: -1 si no hay parámetros de lista (mismo
                // sentinela documentado que en NuevaFactura). Si hay,
                // reutilizamos el ÚNICO handle del proceso, compartido
                // con Ventas (ver `MicrosipState::pl_handle`).
                let pl_handle = if params.lista_parametros.is_empty() {
                    -1
                } else {
                    let handle = match guard.pl_handle {
                        Some(h) => h,
                        None => {
                            let h = new_pl();
                            if h <= 0 {
                                let msg = self
                                    .cc_last_error_message()
                                    .unwrap_or_else(|| "(sin mensaje)".into());
                                return Err(IskandarError::Connection(format!(
                                    "NewPL() falló (handle={h}): {msg}"
                                )));
                            }
                            guard.pl_handle = Some(h);
                            h
                        }
                    };
                    self.check_cxc(pl_clear(handle))?;
                    for (nombre, valor) in &params.lista_parametros {
                        let c_nombre = cstring(nombre)?;
                        let c_valor = cstring_ansi(valor)?;
                        self.check_cxc(pl_set_param(
                            handle,
                            c_nombre.as_ptr(),
                            c_valor.as_ptr(),
                        ))?;
                    }
                    handle
                };

                let c_fecha = cstring(&params.fecha)?;
                let c_folio = cstring(&params.folio)?;
                let c_descripcion = cstring_ansi(&params.descripcion)?;

                tracing::debug!(
                    "NuevoCreditoCc(concepto={}, cliente={}, folio={:?})",
                    params.concepto_cc_id,
                    params.cliente_id,
                    params.folio
                );
                self.check_cxc(nuevo_credito_cc(
                    params.concepto_cc_id,
                    c_fecha.as_ptr(),
                    c_folio.as_ptr(),
                    params.cliente_id,
                    c_descripcion.as_ptr(),
                    params.cobrador_id,
                    pl_handle,
                ))?;

                // TipoImporte fijo en 'R' (ver nota junto a
                // FnRenglonCreditoCc). Impuesto/IvaRetenido/IsrRetenido
                // descontinuados: siempre 0.0. Sin llamadas a
                // RenglonCreditoImpuestoCc (fuera de alcance v1).
                let c_tipo_importe = cstring("R")?;
                if params.aplicaciones.is_empty() {
                    return Err(IskandarError::Validation(
                        "un crédito necesita al menos una aplicación contra un cargo \
                         (RenglonCreditoCc no se llamaría nunca; AplicaCreditoCc \
                         fallaría con un crédito sin renglones)"
                            .into(),
                    ));
                }
                for aplicacion in &params.aplicaciones {
                    let c_folio_cargo = cstring(&aplicacion.folio_cargo)?;
                    tracing::debug!(
                        "RenglonCreditoCc(cargo={}, folio_cargo={:?}, importe={})",
                        aplicacion.cargo_id,
                        aplicacion.folio_cargo,
                        aplicacion.importe
                    );
                    self.check_cxc(renglon_credito_cc(
                        c_tipo_importe.as_ptr(),
                        aplicacion.cargo_id,
                        c_folio_cargo.as_ptr(),
                        aplicacion.importe,
                        0.0, // Impuesto: descontinuado (Refer.md L567)
                        0.0, // IvaRetenido: descontinuado
                        0.0, // IsrRetenido: descontinuado
                        aplicacion.dscto_ppag,
                    ))?;
                }

                // IMPORTANTE: GetDoctoCCId solo es válido MIENTRAS el
                // crédito está "en proceso", es decir, ANTES de
                // AplicaCreditoCc (ver nota en FnGetDoctoCcId arriba).
                let mut docto_id: i32 = 0;
                self.check_cxc(get_docto_cc_id(&mut docto_id))?;

                tracing::debug!("AplicaCreditoCc()");
                self.check_cxc(aplica_credito_cc())?;

                Ok(docto_id)
            }
        })();

        if resultado.is_err() {
            tracing::warn!(
                "crear_credito falló ({:?}); ejecutando AbortaDoctoCxc para no dejar \
                 un documento a medio registrar",
                resultado.as_ref().err()
            );
            // SAFETY: AbortaDoctoCxc tiene la misma semántica documentada
            // que AbortaDoctoVentas: segura de llamar incluso sin
            // documento en proceso, cualquier error es ignorado — no
            // puede enmascarar el error original porque no propaga
            // ninguno.
            unsafe { aborta_docto_cxc() };
        }

        // Cerramos las conexiones pase lo que pase; ignoramos errores de
        // desconexión para no enmascarar el error original de
        // `resultado` (que ya tiene prioridad). Metadatos se cierra antes
        // que empresa por simetría con el orden de apertura, aunque el
        // orden de cierre no es significativo aquí.
        if let Some(meta) = metadatos {
            self.disconnect_internal(meta).ok();
        }
        self.disconnect_internal(empresa).ok();

        resultado
    }

    // --- Internos ---

    fn guard(&self) -> Result<std::sync::MutexGuard<'_, MicrosipState>> {
        self.lock.lock().map_err(|_| {
            IskandarError::Connection(
                "lock de la DLL envenenado: un hilo anterior falló dentro de ApiMicrosip".into(),
            )
        })
    }

    fn symbol<T>(&self, name: &[u8]) -> Result<Symbol<'_, T>> {
        // SAFETY: el tipo T declarado debe corresponder a la firma real
        // exportada por la DLL — por eso las firmas viven todas juntas
        // arriba de este archivo, contra la referencia oficial.
        unsafe { self.lib.get(name) }.map_err(|e| {
            IskandarError::Connection(format!(
                "símbolo '{}' no encontrado en la DLL: {e}",
                String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
            ))
        })
    }

    /// Convierte un código de retorno nativo en `Result`, recuperando
    /// el mensaje de error de la propia DLL cuando hay falla.
    fn check(&self, rc: ErrCode) -> Result<()> {
        if rc == 0 {
            return Ok(());
        }
        let message = self
            .last_error_message()
            .unwrap_or_else(|| "(sin mensaje de la DLL)".to_string());
        Err(IskandarError::Provider { code: rc, message })
    }

    fn last_error_message(&self) -> Option<String> {
        let get_msg: Symbol<FnGetLastErrorMessage> =
            self.symbol(b"GetLastErrorMessage\0").ok()?;
        let mut buf = vec![0u8; 1024];
        // SAFETY: buffer de 1024 bytes válido durante la llamada.
        // La DLL escribe hasta el null-terminator; leemos hasta él abajo.
        unsafe { get_msg(buf.as_mut_ptr() as *mut c_char) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        // Nota: la DLL puede devolver Windows-1252; si aparecen acentos
        // rotos, decodificar con encoding_rs en lugar de UTF-8.
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }

    /// Convierte un código de retorno de una función del módulo de
    /// Ventas en `Result`, recuperando el mensaje con
    /// `veGetLastErrorMessage` — DISTINTO de `check()`/`last_error_message()`
    /// de arriba, que son de la Api básica (`GetLastErrorMessage`). El
    /// módulo de Ventas tiene su propio par de funciones de error.
    fn check_ventas(&self, rc: ErrCode) -> Result<()> {
        if rc == 0 {
            return Ok(());
        }
        let message = self
            .ve_last_error_message()
            .unwrap_or_else(|| "(sin mensaje de veGetLastErrorMessage)".to_string());
        Err(IskandarError::Provider { code: rc, message })
    }

    fn ve_last_error_message(&self) -> Option<String> {
        let get_msg: Symbol<FnVeGetLastErrorMessage> =
            self.symbol(b"veGetLastErrorMessage\0").ok()?;
        let mut buf = vec![0u8; 1024];
        // SAFETY: buffer de 1024 bytes válido durante la llamada, mismo
        // patrón que last_error_message().
        unsafe { get_msg(buf.as_mut_ptr() as *mut c_char) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }

    /// Convierte un código de retorno de una función del módulo de Cxc en
    /// `Result`, recuperando el mensaje con `ccGetLastErrorMessage` —
    /// DISTINTO de `check()`/`last_error_message()` (Api básica) Y de
    /// `check_ventas()`/`ve_last_error_message()` (módulo de Ventas). El
    /// módulo de Cxc tiene su propio tercer par de funciones de error.
    fn check_cxc(&self, rc: ErrCode) -> Result<()> {
        if rc == 0 {
            return Ok(());
        }
        let message = self
            .cc_last_error_message()
            .unwrap_or_else(|| "(sin mensaje de ccGetLastErrorMessage)".to_string());
        Err(IskandarError::Provider { code: rc, message })
    }

    fn cc_last_error_message(&self) -> Option<String> {
        let get_msg: Symbol<FnCcGetLastErrorMessage> =
            self.symbol(b"ccGetLastErrorMessage\0").ok()?;
        let mut buf = vec![0u8; 1024];
        // SAFETY: buffer de 1024 bytes válido durante la llamada, mismo
        // patrón que last_error_message()/ve_last_error_message().
        unsafe { get_msg(buf.as_mut_ptr() as *mut c_char) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }
}

fn cstring(s: &str) -> Result<CString> {
    CString::new(s)
        .map_err(|_| IskandarError::Validation(format!("string con NUL interno: {s:?}")))
}

/// SUPUESTO NO VERIFICADO: los parámetros de texto libre del módulo de
/// Ventas (Descripcion, Notas, OrdenCompra, DescripcionCobro, y los
/// valores de texto de `PLSetParamValue`) se declaran `PAnsiChar` en
/// `ApiMspVentasExt.pas` (no `PChar`/Unicode) — por definición de interop
/// Delphi eso es el codepage ANSI del sistema, Windows-1252 en una
/// instalación es-MX. Pasar UTF-8 crudo produciría mojibake en acentos
/// (á, é, í, ó, ú, ñ, ü) al guardarse en la base. Los caracteres no
/// representables en 1252 se reemplazan (vía encoding_rs, sin pánico) en
/// vez de fallar la llamada.
/// PENDIENTE: confirmar contra una BD de PRUEBAS (nunca producción) que
/// los acentos se almacenan correctos; si la dll resultara aceptar UTF-8,
/// revertir esta función a bytes crudos es un cambio de una línea.
fn cstring_ansi(s: &str) -> Result<CString> {
    let (encoded, _, _tuvo_caracteres_no_representables) = WINDOWS_1252.encode(s);
    CString::new(encoded.into_owned())
        .map_err(|_| IskandarError::Validation(format!("string con NUL interno: {s:?}")))
}
