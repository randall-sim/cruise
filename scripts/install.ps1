$ErrorActionPreference = 'Stop'
$cruiseCheckout = [System.IO.Path]::GetFullPath((Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).ProviderPath).TrimEnd('\')
if ($cruiseCheckout -match '^\\\\(?:wsl\.localhost|wsl\$)\\([^\\]+)\\(.*)$') {
    $cruiseDistribution = $Matches[1]
    $cruiseLinuxPath = '/' + $Matches[2].Replace('\', '/')
    # The generated cmd launcher contains only these checked paths and forwarded arguments.
    if ($cruiseDistribution -match '["%!\r\n]' -or $cruiseLinuxPath -match '["%!\r\n]') {
        throw 'Unsupported character in WSL checkout path.'
    }
    & wsl.exe --distribution $cruiseDistribution --cd $cruiseLinuxPath --exec sh -c 'exec "$HOME/.cargo/bin/cargo" build --locked --release'
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
    $cruiseBin = Join-Path $env:USERPROFILE '.local\bin'
    New-Item -ItemType Directory -Force -Path $cruiseBin | Out-Null
    $cruiseLauncher = '@powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0cruise.ps1" %*' + "`r`n"
    [System.IO.File]::WriteAllText((Join-Path $cruiseBin 'cruise.cmd'), $cruiseLauncher)
    $cruisePowerShell = "& wsl.exe --distribution '" + $cruiseDistribution.Replace("'", "''") + "' --cd '" + $cruiseLinuxPath.Replace("'", "''") + "' --exec './target/release/cruise' @args`r`nexit `$LASTEXITCODE`r`n"
    [System.IO.File]::WriteAllText((Join-Path $cruiseBin 'cruise.ps1'), $cruisePowerShell)
} else {
    & cargo install --locked --path $cruiseCheckout --root (Join-Path $env:USERPROFILE '.local') --force
    if ($LASTEXITCODE -ne 0) { throw 'Rust installation failed.' }
    $cruiseBin = Join-Path $env:USERPROFILE '.local\bin'
}
$cruiseUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($cruiseUserPath -split ';') -notcontains $cruiseBin) {
    [Environment]::SetEnvironmentVariable('Path', ($cruiseBin + ';' + $cruiseUserPath), 'User')
}
if (($env:Path -split ';') -notcontains $cruiseBin) { $env:Path = $cruiseBin + ';' + $env:Path }
Write-Host 'Installed cruise. Open a new terminal, then run cruise run.'
