<#
.SYNOPSIS
    Build the fork's Windows installer locally -- the Windows counterpart of
    docker/build.sh.

.DESCRIPTION
    Needs Rust (rustup, MSVC target), Deno and the Visual Studio Build Tools
    with the C++ workload (Microsoft.VisualStudio.BuildTools via winget; the
    2022 winget package's bootstrapper is too old for the current installer
    feed). WebView2 ships with Windows 11.

    Produces the NSIS installer under src-tauri\target\release\bundle\nsis.
    The fork keeps upstream's version number, so install it over the stock
    build and check About if in doubt.

        & .\scripts\build-windows.ps1
        & .\scripts\build-windows.ps1 -Jobs 4
#>
[CmdletBinding()]
param(
    [int]$Jobs = 0,
    # Install the result: silently, per-user, without a desktop shortcut, and
    # restart OpenDeck if it was running.
    [switch]$Install
)

$ErrorActionPreference = 'Stop'
$repo = Resolve-Path (Join-Path $PSScriptRoot '..')

# Tools installed by winget/rustup in this session are not on PATH until a new
# login shell; find them where they live instead.
foreach ($dir in @("$env:USERPROFILE\.cargo\bin",
                   (Split-Path (Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Recurse -Filter deno.exe -ErrorAction SilentlyContinue | Select-Object -First 1).FullName -ErrorAction SilentlyContinue))) {
    if ($dir -and (Test-Path $dir) -and ($env:PATH -split ';') -notcontains $dir) { $env:PATH = "$dir;$env:PATH" }
}
# tauri-winres needs rc.exe from the Windows SDK, which the Build Tools install
# but do not put on PATH outside a developer prompt. A build that reuses cached
# resources hides this until the build script reruns.
if (-not (Get-Command rc.exe -ErrorAction SilentlyContinue)) {
    $rc = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Recurse -Filter rc.exe -ErrorAction SilentlyContinue |
        Where-Object FullName -match '\\x64\\' | Sort-Object FullName -Descending | Select-Object -First 1
    if ($rc) { $env:PATH = "$($rc.DirectoryName);$env:PATH" } else { throw 'rc.exe not found: install a Windows SDK with the Build Tools' }
}
foreach ($tool in 'cargo', 'deno') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "$tool not found" }
}
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere) -or -not (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)) {
    throw 'MSVC build tools not found (winget install Microsoft.VisualStudio.BuildTools with the VCTools workload)'
}
if ($Jobs -gt 0) { $env:CARGO_BUILD_JOBS = "$Jobs" }

Push-Location $repo
try {
    deno install
    if ($LASTEXITCODE -ne 0) { throw "deno install failed ($LASTEXITCODE)" }
    deno task tauri build --bundles nsis
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

Write-Host ''
Write-Host 'artefacts:'
$setup = Get-ChildItem (Join-Path $repo 'src-tauri\target\release\bundle\nsis') -Filter *-setup.exe -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
Get-ChildItem (Join-Path $repo 'src-tauri\target\release\bundle\nsis') -Filter *.exe -ErrorAction SilentlyContinue |
    ForEach-Object { '  {0} ({1:N0} bytes)' -f $_.FullName, $_.Length }

if ($Install) {
    if (-not $setup) { throw 'no installer to install' }
    $exe = Join-Path $env:LOCALAPPDATA 'OpenDeck\opendeck.exe'
    $wasRunning = [bool](Get-Process opendeck -ErrorAction SilentlyContinue)
    if ($wasRunning) { Stop-Process -Name opendeck -Force; Start-Sleep -Seconds 3 }

    # /S silent, /NS no shortcuts. The NSIS template's /NS skips the Start menu
    # entry as well as the desktop one, so the Start menu entry is put back here
    # if it is missing (an update over an existing install keeps it anyway).
    $p = Start-Process $setup.FullName -ArgumentList '/S', '/NS' -PassThru -Wait
    if ($p.ExitCode -ne 0) { throw "installer failed ($($p.ExitCode))" }

    $startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'OpenDeck.lnk'
    if (-not (Test-Path $startMenu)) {
        $lnk = (New-Object -ComObject WScript.Shell).CreateShortcut($startMenu)
        $lnk.TargetPath = $exe
        $lnk.Save()
    }
    foreach ($desktop in [Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('CommonDesktopDirectory')) {
        $d = Join-Path $desktop 'OpenDeck.lnk'
        if (Test-Path $d) { Remove-Item $d -Force -ErrorAction SilentlyContinue }
    }
    Write-Host "installed $($setup.Name) (no desktop shortcut)"

    # Back into the tray (--hide: with fork deviation 8 that means no WebView at
    # all), with the user environment re-read from the registry so OpenDeck sees
    # variables set since this shell started (the deck's label configs).
    if ($wasRunning) {
        foreach ($v in (Get-Item 'HKCU:\Environment').GetValueNames()) {
            Set-Item "env:$v" ([Environment]::GetEnvironmentVariable($v, 'User'))
        }
        Start-Process $exe -ArgumentList '--hide'
    }
}
