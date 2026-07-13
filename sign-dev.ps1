# sign-dev.ps1 — Firma los binarios compilados con el certificado de desarrollo.
# Usar después de `cargo build` para que SAC permita ejecutarlos.
#
# Requiere PowerShell elevado (administrador).
# El certificado "Iskandar Dev Signing" debe estar en Cert:\LocalMachine\My

param(
    [string]$Profile = "debug"
)

$THUMBPRINT = "7DC2E59B206D1539D4FEE7B601DED80B47B2B61D"
$TARGET     = "$PSScriptRoot\target\i686-pc-windows-msvc\$Profile"

$cert = Get-Item "Cert:\LocalMachine\My\$THUMBPRINT" -ErrorAction Stop

$binaries = Get-ChildItem $TARGET -Filter "*.exe" -ErrorAction SilentlyContinue
if (-not $binaries) {
    Write-Warning "No se encontraron .exe en $TARGET — asegúrate de haber corrido 'cargo build'."
    exit 1
}

foreach ($bin in $binaries) {
    $result = Set-AuthenticodeSignature -FilePath $bin.FullName -Certificate $cert -HashAlgorithm SHA256
    if ($result.Status -eq "Valid") {
        Write-Host "OK  $($bin.Name)" -ForegroundColor Green
    } else {
        Write-Host "FAIL $($bin.Name): $($result.StatusMessage)" -ForegroundColor Red
    }
}

Write-Host "`nFirma completada. Binarios listos para ejecutar." -ForegroundColor Cyan
