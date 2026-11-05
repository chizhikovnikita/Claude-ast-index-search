[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$guard = Join-Path $PSScriptRoot 'check-push-target.sh'
$installer = Join-Path $PSScriptRoot 'install-push-guard.ps1'
$shellCommand = Get-Command sh -ErrorAction SilentlyContinue
if ($shellCommand) {
    $shell = $shellCommand.Source
} else {
    $gitDirectory = Split-Path -Parent (Split-Path -Parent (Get-Command git).Source)
    $shell = Join-Path $gitDirectory 'bin/sh.exe'
}
if (-not (Test-Path -LiteralPath $shell)) { throw 'Git sh is required for the hook tests.' }

function Invoke-Hook([string]$Hook, [string]$Destination, [string]$InputText = '') {
    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $shell
    $startInfo.WorkingDirectory = (Get-Location).ProviderPath
    $arguments = @($Hook, 'origin', $Destination)
    $startInfo.Arguments = ($arguments | ForEach-Object { '"' + $_.Replace('"', '\"') + '"' }) -join ' '
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $startInfo
    $null = $process.Start()
    $process.StandardInput.Write($InputText)
    $process.StandardInput.Close()
    $output = $process.StandardOutput.ReadToEnd()
    $errors = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    $result = [pscustomobject]@{ ExitCode = $process.ExitCode; Output = $output; Errors = $errors }
    $process.Dispose()
    return $result
}

function Write-TestFile([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text.Replace("`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
}

$forkUrl = 'https://github.com/chizhikovnikita/Claude-ast-index-search.git'
$allowed = @($forkUrl, $forkUrl.Substring(0, $forkUrl.Length - 4),
    'https://GITHUB.COM/chizhikovnikita/Claude-ast-index-search.git/',
    'git@github.com:chizhikovnikita/Claude-ast-index-search.git',
    'ssh://git@github.com/chizhikovnikita/Claude-ast-index-search.git',
    'ssh://git@github.com:22/chizhikovnikita/Claude-ast-index-search.git',
    'ssh://git@ssh.github.com:443/chizhikovnikita/Claude-ast-index-search.git')
$blocked = @('https://github.com/defendend/Claude-ast-index-search.git',
    'git@github.com:defendend/Claude-ast-index-search.git',
    'ssh://git@github.com/defendend/Claude-ast-index-search.git',
    'https://github.com/chizhikovnikita/another-repo.git',
    'https://github.com.attacker.invalid/chizhikovnikita/Claude-ast-index-search.git',
    'https://github.com/chizhikovnikita/Claude-ast-index-search.git/extra',
    'https://github.com/chizhikovnikita/Claude-ast-index-search.git?redirect=upstream', '')
foreach ($url in $allowed) {
    $result = Invoke-Hook $guard $url
    if ($result.ExitCode -ne 0) { throw "Allowed fork destination was rejected: $url $($result.Errors)" }
}
foreach ($url in $blocked) {
    $result = Invoke-Hook $guard $url
    if ($result.ExitCode -eq 0) { throw "Unsafe destination was accepted: $url" }
}

$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testRoot = Join-Path $tempBase ('ast-index-push-guard-tests-' + [Guid]::NewGuid().ToString('N'))
$savedTestOutput = $env:AST_INDEX_HOOK_TEST_OUTPUT
$previousLocation = Get-Location
try {
    New-Item -ItemType Directory -Path (Join-Path $testRoot 'scripts') -Force | Out-Null
    Copy-Item -LiteralPath $guard -Destination (Join-Path $testRoot 'scripts/check-push-target.sh')
    & git -c init.templateDir= init -q $testRoot
    if ($LASTEXITCODE -ne 0) { throw 'Cannot create isolated Git fixture.' }
    & git -C $testRoot config core.hooksPath '.git/hooks'
    if ($LASTEXITCODE -ne 0) { throw 'Cannot isolate fixture hooks from global Git settings.' }
    & $installer -ProjectRoot $testRoot
    Set-Location -LiteralPath $testRoot
    $defaultHook = Join-Path $testRoot '.git/hooks/pre-push'
    $result = Invoke-Hook $defaultHook $forkUrl
    if ($result.ExitCode -ne 0) { throw "Fresh hook rejected the fork: $($result.Errors)" }
    $result = Invoke-Hook $defaultHook $blocked[0]
    if ($result.ExitCode -eq 0) { throw 'Fresh hook allowed upstream.' }
    & git -C $testRoot config core.hooksPath '.custom-hooks'
    if ($LASTEXITCODE -ne 0) { throw 'Cannot configure isolated hook fixture.' }
    $hookDirectory = Join-Path $testRoot '.custom-hooks'
    New-Item -ItemType Directory -Path $hookDirectory -Force | Out-Null
    $hook = Join-Path $hookDirectory 'pre-push'
    Write-TestFile $hook @'
#!/bin/sh
printf '%s\n' "$1" "$2" > "$AST_INDEX_HOOK_TEST_OUTPUT"
cat >> "$AST_INDEX_HOOK_TEST_OUTPUT"
exit 23
'@
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { & chmod +x $hook }
    & $installer -ProjectRoot $testRoot
    if ($LASTEXITCODE -ne 0) { throw 'Hook installer failed.' }
    $firstHash = (Get-FileHash -LiteralPath $hook -Algorithm SHA256).Hash
    & $installer -ProjectRoot $testRoot
    if ((Get-FileHash -LiteralPath $hook -Algorithm SHA256).Hash -ne $firstHash) {
        throw 'Repeated installation changed the managed hook.'
    }
    if ((& git -C $testRoot config core.hooksPath) -ne '.custom-hooks') {
        throw 'Installer changed core.hooksPath.'
    }
    if (@(Get-ChildItem -LiteralPath $hookDirectory -Filter 'pre-push.before-ast-index-*').Count -ne 1) {
        throw 'The previous hook was not preserved exactly once.'
    }
    Set-Location -LiteralPath $testRoot
    $env:AST_INDEX_HOOK_TEST_OUTPUT = Join-Path $testRoot 'previous-hook-output.txt'
    $inputText = "refs/heads/main 111 refs/heads/main 222`n"
    $result = Invoke-Hook $hook $forkUrl $inputText
    if ($result.ExitCode -ne 23) { throw "Previous hook exit code was lost: $($result.Errors)" }
    $forwarded = [IO.File]::ReadAllText($env:AST_INDEX_HOOK_TEST_OUTPUT)
    if (-not $forwarded.Contains("origin`n$forkUrl`n$inputText")) {
        throw 'Previous hook did not receive the arguments and stdin.'
    }
    $env:AST_INDEX_HOOK_TEST_OUTPUT = Join-Path $testRoot 'blocked-hook-output.txt'
    $result = Invoke-Hook $hook $blocked[0] $inputText
    if ($result.ExitCode -eq 0 -or (Test-Path -LiteralPath $env:AST_INDEX_HOOK_TEST_OUTPUT)) {
        throw 'Blocked pushes must not reach the previous hook.'
    }
    Remove-Item -LiteralPath (Join-Path $testRoot 'scripts/check-push-target.sh')
    $result = Invoke-Hook $hook $forkUrl
    if ($result.ExitCode -eq 0) { throw 'Missing tracked guard must block publication.' }
} finally {
    Set-Location -LiteralPath $previousLocation
    [Environment]::SetEnvironmentVariable('AST_INDEX_HOOK_TEST_OUTPUT', $savedTestOutput, 'Process')
    $resolvedTestRoot = [IO.Path]::GetFullPath($testRoot)
    if ($resolvedTestRoot.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $resolvedTestRoot) -match '^ast-index-push-guard-tests-[0-9a-f]{32}$') {
        if (Test-Path -LiteralPath $resolvedTestRoot) { Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force }
    } else {
        throw 'Refusing to remove a fixture outside the expected temporary directory.'
    }
}
Write-Output "Push guard verified: $($allowed.Count) allowed and $($blocked.Count) blocked destinations; existing hook, stdin, exit code, custom hooks path, idempotence, and missing-guard protection."
