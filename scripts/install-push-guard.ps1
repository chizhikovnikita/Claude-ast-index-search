[CmdletBinding()]
param(
    [string]$ProjectRoot = (Split-Path -Parent $PSScriptRoot)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = & git -C $ProjectRoot rev-parse --show-toplevel
if ($LASTEXITCODE -ne 0) { throw 'ProjectRoot must be a Git working tree.' }
$repoRoot = [IO.Path]::GetFullPath($repoRoot)
$guardPath = Join-Path $repoRoot 'scripts/check-push-target.sh'
if (-not (Test-Path -LiteralPath $guardPath -PathType Leaf)) {
    throw 'The tracked push destination guard is missing.'
}

$configuredHooks = & git -C $repoRoot config --path --get core.hooksPath
if ($LASTEXITCODE -gt 1) { throw 'Cannot read core.hooksPath.' }
if ($configuredHooks) {
    if ([IO.Path]::IsPathRooted($configuredHooks)) {
        $hooksDirectory = [IO.Path]::GetFullPath($configuredHooks)
    } else {
        $hooksDirectory = [IO.Path]::GetFullPath((Join-Path $repoRoot $configuredHooks))
    }
} else {
    $hooksDirectory = & git -C $repoRoot rev-parse --path-format=absolute --git-path hooks
    if ($LASTEXITCODE -ne 0) { throw 'Cannot find the Git hooks directory.' }
}
New-Item -ItemType Directory -Path $hooksDirectory -Force | Out-Null
$hookPath = Join-Path $hooksDirectory 'pre-push'
$marker = '# ast-index-fork-push-guard:v1'
$previousName = $null
if (Test-Path -LiteralPath $hookPath) {
    if (-not (Test-Path -LiteralPath $hookPath -PathType Leaf)) {
        throw 'The existing pre-push hook is not a regular file.'
    }
    $existing = [IO.File]::ReadAllText($hookPath)
    if ($existing.Contains($marker)) {
        Write-Output "Push destination guard already installed: $hookPath"
        exit 0
    }
    $previousName = 'pre-push.before-ast-index-' + [Guid]::NewGuid().ToString('N')
}

$wrapper = @'
#!/bin/sh
# ast-index-fork-push-guard:v1
set -eu
project_root=$(git rev-parse --show-toplevel)
guard="$project_root/scripts/check-push-target.sh"
if [ ! -f "$guard" ]; then
    echo 'Push blocked: tracked destination guard is missing from this checkout.' >&2
    exit 1
fi
sh "$guard" "$@"
'@
if ($previousName) {
    # The saved hook still receives the original arguments and unconsumed stdin.
    $wrapper += "`n" + 'previous_hook="$(dirname "$0")/' + $previousName + '"' + "`n"
    $wrapper += 'exec "$previous_hook" "$@"' + "`n"
} else {
    $wrapper += "`nexit 0`n"
}
$pendingHook = Join-Path $hooksDirectory ('.pre-push-ast-index-' + [Guid]::NewGuid().ToString('N'))
try {
    [IO.File]::WriteAllText($pendingHook, $wrapper.Replace("`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        & chmod +x $pendingHook
        if ($LASTEXITCODE -ne 0) { throw 'Cannot mark the hook executable.' }
    }
    if ($previousName) {
        Move-Item -LiteralPath $hookPath -Destination (Join-Path $hooksDirectory $previousName)
    }
    Move-Item -LiteralPath $pendingHook -Destination $hookPath
} catch {
    if ($previousName -and -not (Test-Path -LiteralPath $hookPath)) {
        $backup = Join-Path $hooksDirectory $previousName
        if (Test-Path -LiteralPath $backup) { Move-Item -LiteralPath $backup -Destination $hookPath }
    }
    throw
} finally {
    if (Test-Path -LiteralPath $pendingHook) { Remove-Item -LiteralPath $pendingHook }
}
Write-Output "Installed push destination guard: $hookPath"
if ($previousName) { Write-Output "Preserved existing hook: $previousName" }
