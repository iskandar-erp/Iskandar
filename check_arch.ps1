$path = 'C:\Users\Mario Quintana\Downloads\ApiMicrosip2026\ApiMicrosip.dll'
if (!(Test-Path $path)) {
    Write-Host "Error: No se encontró el archivo en $path"
    exit 1
}
$stream = [System.IO.File]::OpenRead($path)
$buffer = New-Object byte[] 4096
$stream.Read($buffer, 0, 4096) | Out-Null
$stream.Close()

$peOffset = [BitConverter]::ToUInt32($buffer, 0x3C)
$machine = [BitConverter]::ToUInt16($buffer, $peOffset + 4)

Write-Host "--- Diagnóstico de Arquitectura ---"
if ($machine -eq 0x14C) {
    Write-Host "DLL: 32-bit (x86)"
} elseif ($machine -eq 0x8664) {
    Write-Host "DLL: 64-bit (x64)"
} else {
    Write-Host "DLL: Desconocida (0x$($machine.ToString('X')))"
}

$exePath = "target\debug\iskandar.exe"
if (Test-Path $exePath) {
    $stream = [System.IO.File]::OpenRead($exePath)
    $stream.Read($buffer, 0, 4096) | Out-Null
    $stream.Close()
    $peOffset = [BitConverter]::ToUInt32($buffer, 0x3C)
    $machine = [BitConverter]::ToUInt16($buffer, $peOffset + 4)
    if ($machine -eq 0x14C) {
        Write-Host "EXE: 32-bit (x86)"
    } elseif ($machine -eq 0x8664) {
        Write-Host "EXE: 64-bit (x64)"
    }
} else {
    Write-Host "EXE: No encontrado (ejecuta cargo build primero)"
}
