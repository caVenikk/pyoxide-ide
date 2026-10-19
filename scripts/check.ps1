# The checks of CI on Windows: runs scripts/check.sh with the bash of Git for
# Windows, as the Windows job of CI does, so there is one implementation.
# Same arguments as check.sh (`--quick`, step names, `--help`).
#
#     powershell -File scripts/check.ps1 [--quick] [STEP...]
$ErrorActionPreference = 'Stop'

# Git for Windows' bash, found above git.exe (Git\cmd\git.exe or
# Git\mingw64\bin\git.exe -> Git\bin\bash.exe); not the WSL bash in System32.
$bash = $null
$git = Get-Command git -ErrorAction SilentlyContinue
if ($git) {
    $dir = Split-Path $git.Source
    for ($i = 0; $i -lt 3 -and $dir -and -not $bash; $i++) {
        $dir = Split-Path $dir
        if ($dir -and (Test-Path "$dir\bin\bash.exe")) { $bash = "$dir\bin\bash.exe" }
    }
}
if (-not $bash) {
    Write-Error 'check.ps1: bash of Git for Windows not found (install Git for Windows)'
}

& $bash (Join-Path $PSScriptRoot 'check.sh') @args
exit $LASTEXITCODE
