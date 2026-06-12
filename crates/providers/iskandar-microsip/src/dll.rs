//! Wrapper sobre `ApiMicrosip.dll` — el ÚNICO módulo del provider con
//! `unsafe`. Todo lo que está arriba de este archivo es Rust seguro.
//!
//! Datos duros de la DLL (según la referencia oficial de Microsip):
//!
//! - DLL Win32 con funciones de convención `stdcall`. Usamos
//!   `extern "system"`, que es stdcall en x86 y la convención C en x64,
//!   cubriendo builds de 32 y 64 bits.
//! - Strings como PChar (null-terminated, estilo C).
//! - Toda función regresa un código de error (`0` = éxito); el detalle
//!   se recupera con `GetLastErrorMessage`.
//! - La DLL es síncrona y guarda estado global interno (documento en
//!   proceso, último error), así que TODO acceso se serializa con un
//!   `Mutex`. Nunca dos hilos dentro de la DLL a la vez.
//!
//! Las firmas declaradas aquí siguen la referencia en
//! `Microsip/Convertir_Markdown/`; quedan pendientes de validar contra
//! los headers oficiales (`ApiMspBasicaExt.cs` / `.pas`) en el primer
//! test de integración contra la DLL real.

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Mutex;

use iskandar_core::{IskandarError, Result};
use libloading::{Library, Symbol};

/// Código de error nativo de la API (0 = sin error).
pub type ErrCode = i32;

// --- Firmas de la API Básica (ApiMspBasicaExt) ---
type FnSetErrorHandling = unsafe extern "system" fn(modo: i32, muestra_dialogos: i32) -> ErrCode;
type FnNewDb = unsafe extern "system" fn(handle: *mut i32) -> ErrCode;
type FnDbConnect = unsafe extern "system" fn(
    handle: i32,
    path: *const c_char,
    usuario: *const c_char,
    password: *const c_char,
) -> ErrCode;
type FnDbDisconnect = unsafe extern "system" fn(handle: i32) -> ErrCode;
type FnGetLastErrorMessage = unsafe extern "system" fn(buffer: *mut c_char, len: i32) -> ErrCode;

/// Handle opaco de conexión que entrega la DLL.
#[derive(Debug, Clone, Copy)]
pub struct DbHandle(pub(crate) i32);

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
    /// Flujo según la referencia: `SetErrorHandling(0,0)` (modo códigos
    /// de retorno, sin diálogos) → `NewDB` → `DBConnect`.
    pub fn connect(&self, db_path: &str, usuario: &str, password: &str) -> Result<DbHandle> {
        let _guard = self.guard()?;

        let set_error_handling: Symbol<FnSetErrorHandling> = self.symbol(b"SetErrorHandling\0")?;
        let new_db: Symbol<FnNewDb> = self.symbol(b"NewDB\0")?;
        let db_connect: Symbol<FnDbConnect> = self.symbol(b"DBConnect\0")?;

        let c_path = cstring(db_path)?;
        let c_user = cstring(usuario)?;
        let c_pass = cstring(password)?;

        let mut handle: i32 = 0;
        // SAFETY: las firmas corresponden a la referencia oficial; los
        // punteros (handle de salida y CStrings) viven durante toda la
        // llamada.
        unsafe {
            set_error_handling(0, 0);
            self.check(new_db(&mut handle))?;
            self.check(db_connect(
                handle,
                c_path.as_ptr(),
                c_user.as_ptr(),
                c_pass.as_ptr(),
            ))?;
        }
        Ok(DbHandle(handle))
    }

    pub fn disconnect(&self, handle: DbHandle) -> Result<()> {
        let _guard = self.guard()?;
        let db_disconnect: Symbol<FnDbDisconnect> = self.symbol(b"DBDisconnect\0")?;
        // SAFETY: handle obtenido de connect(); firma según referencia.
        let rc = unsafe { db_disconnect(handle.0) };
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
        // SAFETY: buffer válido del tamaño indicado durante la llamada.
        unsafe { get_msg(buf.as_mut_ptr() as *mut c_char, buf.len() as i32) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        // Nota: la DLL probablemente regresa Windows-1252; si aparecen
        // acentos rotos, decodificar con encoding_rs en lugar de UTF-8.
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }
}

fn cstring(s: &str) -> Result<CString> {
    CString::new(s)
        .map_err(|_| IskandarError::Validation(format!("string con NUL interno: {s:?}")))
}
