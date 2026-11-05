[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('Build', 'Test', 'Install', 'Index')]
    [string]$Command = 'Build',
    [string]$CompilerBin = $env:AST_INDEX_MINGW_BIN,
    [string]$TargetDirectory = $env:CARGO_TARGET_DIR,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ExtraArguments = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-Native {
    param([string]$Executable, [string[]]$Arguments)
    $previousPreference = $ErrorActionPreference
    try {
        # Windows PowerShell treats redirected native stderr as ErrorRecord,
        # including successful cargo progress. Judge success by the exit code.
        $ErrorActionPreference = 'Continue'
        & $Executable @Arguments
        $nativeExit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($nativeExit -ne 0) { throw "$Executable failed with exit code $nativeExit." }
}
$projectRoot = Split-Path -Parent $PSScriptRoot
$cargoBin = Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cargo/bin'
if (-not $TargetDirectory) {
    if (-not $env:LOCALAPPDATA) { throw 'Set CARGO_TARGET_DIR or -TargetDirectory on this platform.' }
    # Retain the existing local build cache when updating the fork.
    $TargetDirectory = Join-Path $env:LOCALAPPDATA 'ast-index-build/20b4a614'
}
$TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory)
$environmentNames = @('PATH', 'CC', 'CXX', 'AR', 'CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER',
    'CARGO_TARGET_DIR', 'AST_INDEX_CACHE_DIR', 'CARGO_PROFILE_RELEASE_LTO', 'CARGO_PROFILE_RELEASE_CODEGEN_UNITS')
$savedEnvironment = @{}
foreach ($name in $environmentNames) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$previousLocation = Get-Location
try {
    Set-Location -LiteralPath $projectRoot
    $env:PATH = $cargoBin + [IO.Path]::PathSeparator + $env:PATH
    $env:CARGO_TARGET_DIR = $TargetDirectory
    if ($Command -eq 'Index') {
        $binary = Join-Path $TargetDirectory 'release/ast-index.exe'
        if (-not (Test-Path -LiteralPath $binary)) { $binary = Join-Path $cargoBin 'ast-index.exe' }
        if (-not (Test-Path -LiteralPath $binary)) { throw 'Build or install ast-index before indexing.' }
        Invoke-Native $binary (@('rebuild') + $ExtraArguments)
    } else {
        $rustInfo = Invoke-Native 'rustc' @('-vV')
        if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect the active Rust toolchain.' }
        $target = $env:CARGO_BUILD_TARGET
        if (-not $target) {
            $hostLine = $rustInfo | Where-Object { $_ -like 'host: *' } | Select-Object -First 1
            $target = $hostLine.Substring(6)
        }
        if ($target -match '-windows-gnu$') {
            if (-not $CompilerBin) {
                $CompilerBin = Join-Path $env:LOCALAPPDATA 'Programs/ast-index-toolchain/mingw64/bin'
            }
            foreach ($tool in @('gcc.exe', 'g++.exe', 'ar.exe')) {
                if (-not (Test-Path -LiteralPath (Join-Path $CompilerBin $tool))) {
                    throw "Missing $tool; set AST_INDEX_MINGW_BIN or -CompilerBin to a MinGW-w64 bin directory."
                }
            }
            $env:PATH = $CompilerBin + [IO.Path]::PathSeparator + $env:PATH
            $env:CC = Join-Path $CompilerBin 'gcc.exe'
            $env:CXX = Join-Path $CompilerBin 'g++.exe'
            $env:AR = Join-Path $CompilerBin 'ar.exe'
            $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $env:CC
        }
        switch ($Command) {
            'Build' {
                [Environment]::SetEnvironmentVariable('CARGO_PROFILE_RELEASE_LTO', $null, 'Process')
                [Environment]::SetEnvironmentVariable('CARGO_PROFILE_RELEASE_CODEGEN_UNITS', $null, 'Process')
                Invoke-Native 'cargo' (@('build', '--release', '--workspace', '--locked') + $ExtraArguments)
            }
            'Install' {
                [Environment]::SetEnvironmentVariable('CARGO_PROFILE_RELEASE_LTO', $null, 'Process')
                [Environment]::SetEnvironmentVariable('CARGO_PROFILE_RELEASE_CODEGEN_UNITS', $null, 'Process')
                Invoke-Native 'cargo' (@('install', '--path', $projectRoot, '--locked', '--force') + $ExtraArguments)
            }
            'Test' {
                $env:AST_INDEX_CACHE_DIR = Join-Path ([IO.Path]::GetTempPath()) "ast-index-tests-$PID"
                $env:CARGO_PROFILE_RELEASE_LTO = 'false'
                $env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '16'
                Invoke-Native 'cargo' (@('test', '--release', '--workspace', '--locked', '--no-fail-fast') + $ExtraArguments)
            }
        }
    }
    if ($LASTEXITCODE -ne 0) { throw "$Command failed with exit code $LASTEXITCODE." }
} finally {
    foreach ($name in $environmentNames) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
    Set-Location -LiteralPath $previousLocation
}
