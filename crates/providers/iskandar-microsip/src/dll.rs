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

use iskandar_core::{IskandarError, Result};
use libloading::{Library, Symbol};

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
}

pub struct MicrosipDll {
    lib: Library,
    /// La DLL no es thread-safe: un solo hilo dentro a la vez.
    lock: Mutex<()>,
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
            lock: Mutex::new(()),
        })
    }

    /// Abre una conexión a la base de empresa y regresa su handle.
    ///
    /// Flujo según referencia oficial:
    /// `SetErrorHandling(0,0)` → `NewDB()` → `DBConnect(handle, path, user, pass)`
    pub fn connect(&self, db_path: &str, usuario: &str, password: &str) -> Result<DbHandle> {
        let _guard = self.guard()?;

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
        let db_disconnect: Symbol<FnDbDisconnect> = self.symbol(b"DBDisconnect\0")?;
        tracing::debug!("DBDisconnect(db={})", handle.db);
        // SAFETY: handle obtenido de connect(); firma según referencia.
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

    // --- Internos ---

    fn guard(&self) -> Result<std::sync::MutexGuard<'_, ()>> {
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
}

fn cstring(s: &str) -> Result<CString> {
    CString::new(s)
        .map_err(|_| IskandarError::Validation(format!("string con NUL interno: {s:?}")))
}
