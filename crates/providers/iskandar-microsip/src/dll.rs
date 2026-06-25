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
