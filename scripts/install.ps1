# Install quack (TUI and desktop GUI) on Windows.
#
#   irm https://quack.clackstack.dev/install.ps1 | iex
#   & ([scriptblock]::Create((irm .../install.ps1))) -Tui
#   & ([scriptblock]::Create((irm .../install.ps1))) -Gui
#   & ([scriptblock]::Create((irm .../install.ps1))) -Plucked
#
# By default this installs BOTH the TUI (`quack.exe`) and desktop GUI (`quack-gui`),
# which are separate apps that share your config, sessions, memory notes and prompt
# history. Pass -Tui or -Gui to install only one. -Plucked installs the bundle without
# hosted-service provider files (OpenRouter, Claude, OpenAI, ...), for setups with their
# own provider. -BinaryOnly updates the TUI binary and leaves installed defaults alone.
#
# A full install replaces the defaults folder, so files you deleted from it come back.
# To turn a default off for good, override it in your own config instead (for example
# `enabled = false` in %APPDATA%\quack\providers.d\<id>.toml), or update with -BinaryOnly.
[CmdletBinding()]
param(
    # Install only the terminal UI (quack.exe) and its defaults bundle.
    [Alias('Cli')]
    [switch]$Tui,
    # Install only the desktop GUI (quack-gui). (Default with neither switch: install both.)
    [switch]$Gui,
    # Leave out the provider files for hosted services; everything else is installed.
    [switch]$Plucked,
    # Install or update only the TUI binary; existing defaults are left as they are.
    [switch]$BinaryOnly,
    # Install folder; quack.exe and defaults\ go directly in it.
    [string]$Prefix = $(if ($env:QUACK_INSTALL_PREFIX) { $env:QUACK_INSTALL_PREFIX } else { Join-Path $env:LOCALAPPDATA 'Programs\quack' }),
    # A release tag such as v0.2.0; the latest release by default.
    [string]$Version = 'latest',
    # Install from a downloaded quack-*.zip (TUI) or quack-gui-*-setup.exe (GUI).
    [string]$Archive,
    # Build with cargo from this checkout and install that.
    [string]$FromSource,
    # Remove the selected component(s) (both TUI and GUI by default); config, sessions and memory stay.
    [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'
$repo = 'ClackStack/quack'
function Say($m) { Write-Host "quack: $m" }

$explicitTui = $Tui.IsPresent
$explicitGui = $Gui.IsPresent
$wantTui = $explicitTui
$wantGui = $explicitGui
if (-not $wantTui -and -not $wantGui) {
    if ($Archive) {
        if ($Archive -like '*.exe' -or $Archive -like '*.msi') { $wantGui = $true }
        else { $wantTui = $true }
    } else {
        $wantTui = $true
        $wantGui = $true
    }
}

$exe = Join-Path $Prefix 'quack.exe'
$srcGuiExe = Join-Path $Prefix 'quack-gui.exe'
$defaults = Join-Path $Prefix 'defaults'
$nsisDir = Join-Path $env:LOCALAPPDATA 'quack'
$nsisGuiExe = Join-Path $nsisDir 'quack-gui.exe'
$nsisUninst = Join-Path $nsisDir 'uninstall.exe'

function Set-UserPath([string]$dir, [bool]$add) {
    $cur = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @($cur -split ';' | Where-Object { $_ -and ($_.TrimEnd('\') -ne $dir.TrimEnd('\')) })
    if ($add) { $parts += $dir }
    [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
}

if ($Uninstall) {
    if ($wantTui) {
        Remove-Item -Force -ErrorAction SilentlyContinue $exe, "$exe.old"
        Say "removed $exe"
    }
    if ($wantGui) {
        Remove-Item -Force -ErrorAction SilentlyContinue $srcGuiExe, "$srcGuiExe.old"
        if (Test-Path $nsisUninst) {
            Say "uninstalling desktop GUI ($nsisUninst)"
            $p = Start-Process -FilePath $nsisUninst -ArgumentList '/S' -Wait -PassThru
            if ($p.ExitCode -ne 0) { Say "note: GUI uninstaller exited with code $($p.ExitCode)" }
        }
        Set-UserPath $nsisDir $false
        Say "removed desktop GUI"
    }
    if (-not (Test-Path $exe) -and -not (Test-Path $srcGuiExe)) {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $defaults
        Set-UserPath $Prefix $false
        if ((Test-Path $Prefix) -and -not (Get-ChildItem $Prefix)) { Remove-Item $Prefix }
        Say "removed $defaults"
    }
    return
}

# The defaults bundle without the provider files whose base_url is not on this machine,
# as scripts/copy-defaults.sh --plucked builds it for the release archives.
function Copy-PluckedDefaults([string]$from, [string]$to) {
    Copy-Item -Recurse $from $to
    Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $to 'config.toml')
    foreach ($f in Get-ChildItem (Join-Path $to 'providers.d') -Filter *.toml -ErrorAction SilentlyContinue) {
        $m = Select-String -Path $f.FullName -Pattern '^base_url *= *"[a-z]*://([^/:"]*)' | Select-Object -First 1
        $host_ = if ($m) { $m.Matches[0].Groups[1].Value } else { '' }
        if (-not ($host_ -like '127.*' -or $host_ -eq 'localhost' -or $host_ -eq '[::1]')) {
            Remove-Item -Force $f.FullName
        }
    }
}

function Verify-Checksum([string]$sumsFile, [string]$filePath, [string]$assetName) {
    $line = Get-Content $sumsFile | Where-Object { $_ -match "^([0-9a-f]{64}) +\*?$([regex]::Escape($assetName))$" } | Select-Object -First 1
    if (-not $line) { throw "$assetName is not listed in SHA256SUMS" }
    $want = ($line -split ' ')[0]
    $got = (Get-FileHash -Algorithm SHA256 $filePath).Hash.ToLower()
    if ($want -ne $got) { throw "checksum mismatch for $assetName" }
}

function Install-ExeSafe([string]$src, [string]$dest) {
    Remove-Item -Force -ErrorAction SilentlyContinue "$dest.old"
    if (Test-Path $dest) { Move-Item -Force $dest "$dest.old" }
    Copy-Item $src $dest
    Remove-Item -Force -ErrorAction SilentlyContinue "$dest.old"
    Say "installed $dest"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("quack-install-" + [guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    $newTuiBin = $null
    $newGuiBin = $null
    $newDefaults = $null
    $guiSetup = $null

    if ($FromSource) {
        if (-not (Test-Path (Join-Path $FromSource 'Cargo.toml'))) { throw "$FromSource is not a quack checkout" }
        $cargoTarget = if ($env:CARGO_TARGET_DIR) {
            if ([IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) { $env:CARGO_TARGET_DIR } else { Join-Path $FromSource $env:CARGO_TARGET_DIR }
        } else {
            Join-Path $FromSource 'target'
        }
        $uiTool = if (Get-Command pnpm -ErrorAction SilentlyContinue) { 'pnpm' }
                  elseif (Get-Command npm -ErrorAction SilentlyContinue) { 'npm' }
                  else { $null }
        if ($wantGui -and -not $uiTool) {
            if ($explicitGui) { throw 'building the desktop GUI from source requires pnpm or npm' }
            Say 'note: skipping desktop GUI --from-source (requires pnpm or npm); installing TUI'
            $wantGui = $false
        }

        Say "building from $FromSource"
        Push-Location $FromSource
        try {
            if ($wantTui) {
                cargo build --release --locked -p quack-cli
                if ($LASTEXITCODE) { throw 'TUI build failed' }
                $newTuiBin = Join-Path $cargoTarget 'release\quack.exe'
            }
            if ($wantGui) {
                $uiDir = Join-Path $FromSource 'crates\quack-gui\ui'
                Push-Location $uiDir
                try {
                    if ($uiTool -eq 'pnpm') {
                        pnpm install --frozen-lockfile; if ($LASTEXITCODE) { throw 'GUI frontend install failed' }
                        pnpm run build; if ($LASTEXITCODE) { throw 'GUI frontend build failed' }
                    } else {
                        npm ci; if ($LASTEXITCODE) { throw 'GUI frontend install failed' }
                        npm run build; if ($LASTEXITCODE) { throw 'GUI frontend build failed' }
                    }
                } finally { Pop-Location }
                cargo build --release --locked -p quack-gui --features desktop
                if ($LASTEXITCODE) { throw 'GUI build failed' }
                $newGuiBin = Join-Path $cargoTarget 'release\quack-gui.exe'
            }
        } finally { Pop-Location }

        if ($wantTui -or $wantGui) {
            $newDefaults = Join-Path $FromSource 'defaults'
            if ($Plucked) {
                Copy-PluckedDefaults $newDefaults (Join-Path $tmp 'defaults')
                $newDefaults = Join-Path $tmp 'defaults'
            }
        }
    } else {
        if ($Archive) {
            if (-not (Test-Path $Archive)) { throw "no such archive: $Archive" }
            if ($Archive -like '*.exe') {
                if ($explicitTui) { throw "$Archive is a GUI setup installer; pass -Gui or use a .zip for -Tui" }
                $wantTui = $false
                $wantGui = $true
                $guiSetup = $Archive
            } else {
                if ($explicitGui) { throw "$Archive is a TUI archive; pass -Tui or use a -setup.exe for -Gui" }
                $wantTui = $true
                $wantGui = $false
            }
        } else {
            $arch = switch ($env:PROCESSOR_ARCHITECTURE) {
                'AMD64' { 'x86_64' } 'ARM64' { 'aarch64' }
                default { throw "unsupported CPU $env:PROCESSOR_ARCHITECTURE" }
            }
            $base = if ($Version -eq 'latest') { "https://github.com/$repo/releases/latest/download" }
                    else { "https://github.com/$repo/releases/download/$Version" }
            $ProgressPreference = 'SilentlyContinue'
            $sumsPath = Join-Path $tmp 'SHA256SUMS'
            Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile $sumsPath

            if ($wantTui) {
                $tuiFlavour = if ($Plucked) { 'quack-plucked' } else { 'quack' }
                $tuiName = "$tuiFlavour-$arch-pc-windows-msvc.zip"
                $tuiPath = Join-Path $tmp $tuiName
                Say "downloading $tuiName ($Version)"
                Invoke-WebRequest -UseBasicParsing "$base/$tuiName" -OutFile $tuiPath
                Verify-Checksum $sumsPath $tuiPath $tuiName
                $Archive = $tuiPath
            }
            if ($wantGui) {
                $guiFlavour = if ($Plucked) { 'quack-gui-plucked' } else { 'quack-gui' }
                $guiName = "$guiFlavour-$arch-pc-windows-msvc-setup.exe"
                $guiPath = Join-Path $tmp $guiName
                Say "downloading $guiName ($Version)"
                Invoke-WebRequest -UseBasicParsing "$base/$guiName" -OutFile $guiPath
                Verify-Checksum $sumsPath $guiPath $guiName
                $guiSetup = $guiPath
            }
        }

        if ($wantTui) {
            $x = Join-Path $tmp 'x'
            Expand-Archive -Path $Archive -DestinationPath $x
            $top = Get-ChildItem -Directory $x | Select-Object -First 1
            if (-not $top -or -not (Test-Path (Join-Path $top.FullName 'quack.exe'))) { throw "$Archive does not look like a quack archive" }
            $newTuiBin = Join-Path $top.FullName 'quack.exe'
            $d = Join-Path $top.FullName 'defaults'
            if (Test-Path $d) { $newDefaults = $d }
        }
    }

    if ($wantTui -or $newGuiBin) {
        New-Item -ItemType Directory -Force $Prefix | Out-Null
    }
    if ($wantTui) {
        Install-ExeSafe $newTuiBin $exe
    }

    if ($wantTui -or $newGuiBin) {
        # The defaults: replaced as a whole, so files removed upstream disappear too.
        if ($BinaryOnly) {
            if (Test-Path $defaults) { Say "left the defaults in $defaults as they are (-BinaryOnly)" }
            else { Say 'no defaults installed (-BinaryOnly): add providers with `quack setup` or your own files' }
        } elseif (-not $newDefaults) {
            Say "this archive has no defaults bundle; left $defaults as it is"
        } else {
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue "$defaults.new"
            Copy-Item -Recurse $newDefaults "$defaults.new"
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $defaults
            Move-Item "$defaults.new" $defaults
            Say "installed the defaults in $defaults"
        }

        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if (-not (($userPath -split ';') | Where-Object { $_.TrimEnd('\') -eq $Prefix.TrimEnd('\') })) {
            Set-UserPath $Prefix $true
            Say "added $Prefix to your PATH; open a new terminal to use it"
        }
    }

    if ($wantGui) {
        if ($guiSetup) {
            Say "installing desktop GUI ($guiSetup)"
            $proc = Start-Process -FilePath $guiSetup -ArgumentList '/S' -Wait -PassThru
            if ($proc.ExitCode -ne 0) { throw "GUI installer exited with code $($proc.ExitCode)" }
            if (Test-Path $nsisGuiExe) {
                $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
                if (-not (($userPath -split ';') | Where-Object { $_.TrimEnd('\') -eq $nsisDir.TrimEnd('\') })) {
                    Set-UserPath $nsisDir $true
                    Say "added $nsisDir to your PATH for quack-gui"
                }
            }
            Say "installed desktop GUI (Start Menu / $nsisDir)"
        } elseif ($newGuiBin) {
            Install-ExeSafe $newGuiBin $srcGuiExe
        }
    }

    if ($wantTui -and $wantGui) {
        Say 'installed both TUI (`quack`) and GUI (`quack-gui`); both share one config, sessions and memory'
    } elseif ($wantGui) {
        Say 'installed the desktop GUI only (`quack-gui`); re-run without -Gui anytime to add `quack`'
    } else {
        Say 'installed the TUI only (`quack`); re-run without -Tui anytime to add the desktop GUI'
    }

    if ($Plucked) { Say 'no hosted services are set up (-Plucked): add your server with `quack setup` or in Settings' }
    elseif ($wantTui) { Say 'run `quack setup` to pick a model, then `quack` or `quack-gui`' }
    else { Say 'open Quack (or run `quack-gui`) to get started' }
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmp
}
