//! Test de integración real contra `ApiMicrosip.dll`.
//!
//! Requiere Windows, la DLL instalada y una base Firebird accesible.
//! Se configura por variables de entorno y se corre explícitamente:
//!
//! ```text
//! set ISKANDAR_MICROSIP_DLL=C:\Microsip\ApiMicrosip.dll
//! set ISKANDAR_MICROSIP_DB=172.20.10.185:C:\Microsip datos\SF.FDB
//! set ISKANDAR_MICROSIP_USER=SYSDBA
//! set ISKANDAR_MICROSIP_PASS=masterkey
//! cargo test -p iskandar-microsip -- --ignored
//! ```
#![cfg(windows)]

use iskandar_microsip::dll::MicrosipDll;

#[test]
#[ignore = "requiere ApiMicrosip.dll y una base Firebird real"]
fn carga_dll_y_conecta() {
    let dll_path =
        std::env::var("ISKANDAR_MICROSIP_DLL").expect("define ISKANDAR_MICROSIP_DLL");
    let dll = MicrosipDll::load(&dll_path).expect("la DLL debe cargar");

    let db = std::env::var("ISKANDAR_MICROSIP_DB").expect("define ISKANDAR_MICROSIP_DB");
    let user = std::env::var("ISKANDAR_MICROSIP_USER").expect("define ISKANDAR_MICROSIP_USER");
    let pass = std::env::var("ISKANDAR_MICROSIP_PASS").expect("define ISKANDAR_MICROSIP_PASS");

    let handle = dll.connect(&db, &user, &pass).expect("conexión a la base");
    dll.disconnect(handle).expect("desconexión limpia");
}
