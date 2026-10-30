# Linux binary of the launcher, built on Windows with `zig cc` (pip package `ziglang`) as linker and C compiler, glibc 2.28 target.
# Output: dist_linux\rac-rs-launcher  (plus dist_linux\lib-tests\<name> = the cross-built unit-test binary when -Tests is given).
# On a real Linux PC just run `cargo build --release` in launcher\ (gcc/cc); this script is only for building from Windows.
#   & .\build_linux.ps1            release binary
#   & .\build_linux.ps1 -Tests      release binary + lib test binary
#   & .\build_linux.ps1 -Package    release binary + the two release files in dist_linux: rac-rs-launcher-linux-x64 (the bare program: v0.1.2
#                                   launchers update themselves from exactly this asset name, so it stays a release asset) and
#                                   rac-rs-launcher-linux-x64.tar.gz (the download for people: a folder with the launcher (run bit kept),
#                                   README-first-steps.txt and the icon), each with a .sha256 file.
param([switch]$Tests, [switch]$Package, [string]$Glibc = '2.28')
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$tools = Join-Path $here 'linux_tools'
$target = 'x86_64-unknown-linux-gnu'
New-Item -ItemType Directory -Force -Path $tools | Out-Null
$ErrorActionPreference = 'Continue'   # pip / rustup / cargo print progress on stderr; exit codes decide
$have = & python -c "import ziglang" 2>$null; if ($LASTEXITCODE -ne 0) { & python -m pip install ziglang }
$zig = & python -c "import ziglang,os;print(os.path.join(os.path.dirname(ziglang.__file__),'zig.exe'))"
if (-not (Test-Path -LiteralPath $zig)) { throw 'zig not found: run  python -m pip install ziglang' }
# the wrapper script zigld.py is checked in next to this file; the .cmd shims call it
Set-Content -LiteralPath (Join-Path $tools 'cc.cmd') -Encoding ascii -Value "@python `"%~dp0zigld.py`" %*"
Set-Content -LiteralPath (Join-Path $tools 'ar.cmd') -Encoding ascii -Value "@`"$zig`" ar %*"
& rustup target add $target
$free = [math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB, 1)
Write-Host "free RAM: $free GB"
if ($free -lt 6) { throw "less than 6 GB free RAM ($free GB): do other work first" }
$env:RUSTC_WRAPPER = ''
$env:RACL_GLIBC = $Glibc
$env:CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = Join-Path $tools 'cc.cmd'
$env:CC_x86_64_unknown_linux_gnu = Join-Path $tools 'cc.cmd'
$env:AR_x86_64_unknown_linux_gnu = Join-Path $tools 'ar.cmd'
$tdir = Join-Path $here 'target'
$dist = Join-Path $here 'dist_linux'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Push-Location (Join-Path $here 'launcher')
try {
    & cargo build --release --target $target --target-dir $tdir
    if ($LASTEXITCODE -ne 0) { Write-Host 'LINUX BUILD FAILED'; exit $LASTEXITCODE }
    Copy-Item -LiteralPath (Join-Path $tdir "$target\release\rac-rs-launcher") -Destination (Join-Path $dist 'rac-rs-launcher') -Force
    if ($Package) {
        $raw = Join-Path $dist 'rac-rs-launcher-linux-x64'
        $tgz = Join-Path $dist 'rac-rs-launcher-linux-x64.tar.gz'
        Copy-Item -LiteralPath (Join-Path $dist 'rac-rs-launcher') -Destination $raw -Force
        & python -I (Join-Path $tools 'pack_tar.py') $raw (Join-Path $tools 'README-first-steps.txt') (Join-Path $here 'launcher\assets\rac-rs-256.png') $tgz
        if ($LASTEXITCODE -ne 0) { Write-Host 'PACKAGE FAILED'; exit $LASTEXITCODE }
        foreach ($f in @($raw, $tgz)) {
            $h = (Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash.ToLower()
            Set-Content -LiteralPath "$f.sha256" -Encoding ascii -Value "$h  $(Split-Path -Leaf $f)"
            Write-Host "$(Split-Path -Leaf $f)  $h"
        }
    }
    if ($Tests) {
        $out = & cargo test --lib --no-run --target $target --target-dir $tdir --message-format=json 2>$null
        if ($LASTEXITCODE -ne 0) { Write-Host 'LINUX TEST BUILD FAILED'; exit $LASTEXITCODE }
        $exe = $out | Where-Object { $_ -match '"executable":"' } | ForEach-Object { ($_ | ConvertFrom-Json).executable } | Select-Object -Last 1
        New-Item -ItemType Directory -Force -Path (Join-Path $dist 'lib-tests') | Out-Null
        Copy-Item -LiteralPath $exe -Destination (Join-Path $dist 'lib-tests\launcher-lib-tests') -Force
        Write-Host "lib test binary: $exe"
    }
} finally { Pop-Location }
Write-Host "LINUX BUILD OK: $dist\rac-rs-launcher"
