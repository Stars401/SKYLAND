# SKYLAND full verification entry (one command runs everything)
#   .\run-checks.ps1            default: Rust tests + JS syntax + unit checks
#   .\run-checks.ps1 -Browser   also run real-browser verification (needs cargo build first)
#
# Why -Browser exists separately: passing compilation does NOT mean the UI works.
# The provider [Edit] button was once fully green on cargo/node --check while doing
# nothing at runtime, because the listener was destroyed by an innerHTML rebuild.
param([switch]$Browser)

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$web  = Join-Path $root 'skyland-web'
$scripts = Join-Path $web 'scripts'
$failed = New-Object System.Collections.ArrayList

# 必须在调用 cargo/node 之前切到 workspace 根：脚本可能被从任意目录调用，
# 留在调用方目录会让 cargo 报 "could not find Cargo.toml" 而误判为测试失败。
Set-Location $root
# Windows PowerShell 5.1 读重定向的 UTF-8 输出会按 ANSI 解码 → 中文变乱码。
# 显式声明 UTF-8，保证被重定向到日志时也可读。
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}

function Head($t) { Write-Host ''; Write-Host "==== $t ====" -ForegroundColor Cyan }
function Ok($t)   { Write-Host "  [ok] $t" -ForegroundColor Green }
function No($t)   { Write-Host "  [FAIL] $t" -ForegroundColor Red; $null = $failed.Add($t) }

# ---------- 1. Rust ----------
Head 'cargo test --workspace'
# Rust 全量测试约 2-4 分钟；直接 Out-String 会把输出全缓冲，运行时毫无进度、
# 极易被误判为卡死。故先落盘再回读：既保留可解析的完整输出，又能实时看到进度。
# 注意：不要用 `2>&1 | Tee-Object | Out-String` 接管 native 命令的 stderr。
# 在 Windows PowerShell 5.1 下该组合会死锁 —— 管道等不到 EOF，脚本表现为"卡死"。
# 改用 cmd 重定向彻底绕开 PowerShell 的管道缓冲。
# 每次用**唯一**日志名：复用固定名会撞上两种故障 ——
#   ① 上一轮 cargo 仍持有句柄 → 本轮写不进去（cmd 返回 1）；
#   ② 本轮没真正跑完时回读到上一轮"全绿"残留 → **假通过**，比失败更危险。
$logFile = Join-Path $env:TEMP ("skyland-cargo-test-" + [Guid]::NewGuid().ToString('N').Substring(0,8) + ".log")
Write-Host "  (输出写入 $logFile，全量测试需 2-4 分钟)"
& cmd /c "cargo test --workspace --no-fail-fast --offline > `"$logFile`" 2>&1"
$cargoExit = $LASTEXITCODE
$out = if (Test-Path $logFile) { Get-Content $logFile -Raw } else { '' }
$pass = ([regex]::Matches($out, '(\d+) passed') | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
$failN = ([regex]::Matches($out, '(\d+) failed') | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
if ($cargoExit -ne 0 -or $failN -gt 0 -or $pass -le 0) { No "cargo test (exit=$cargoExit passed=$pass failed=$failN)" }
else { Ok "cargo test (exit=0 passed=$pass failed=0)" }

# ---------- 2. JS syntax ----------
Head 'node --check'
$bad = 0
$files = @()
$files += Get-ChildItem (Join-Path $web 'static\js') -Filter *.js
$files += Get-ChildItem $scripts -Filter *.js
foreach ($f in $files) {
  & node --check $f.FullName 2>&1 | Out-Null
  if ($LASTEXITCODE -ne 0) { No "syntax $($f.Name)"; $bad++ }
}
if ($bad -eq 0) { Ok "syntax ($($files.Count) files)" }

# ---------- 3. JS check scripts ----------
$checks = @('structure-check.js','storage-migrate-test.js','ws-tree-test.js','brand-check.js')
foreach ($c in $checks) {
  Head $c
  $o = & node (Join-Path $scripts $c) 2>&1 | Out-String
  $last = ($o.Trim() -split "`r?`n")[-1]
  if ($LASTEXITCODE -ne 0) { No "$c -> $last" } else { Ok "$last" }
}

# ---------- 4. Browser (opt-in) ----------
if ($Browser) {
  $exe = Join-Path $root 'target\debug\skyland-web.exe'
  if (-not (Test-Path $exe)) {
    Write-Host "  [skip] browser: $exe not found, run cargo build -p skyland-web first" -ForegroundColor Yellow
  } else {
    Head 'browser verification'
    # 浏览器用例会写入夹具数据，必须隔离到临时数据目录 ——
    # 否则验证一次就把用户真实的 %APPDATA%\SKYLAND 搅上测试会话。
    $tmpData = Join-Path $env:TEMP ("skyland-browsertest-" + [Guid]::NewGuid().ToString('N').Substring(0,8))
    New-Item -ItemType Directory -Force -Path $tmpData | Out-Null
    $env:SKYLAND_DATA_DIR = $tmpData
    $env:SKYLAND_DESKTOP_MODE = '1'
    $proc = Start-Process -FilePath $exe -WorkingDirectory $root -WindowStyle Hidden -PassThru `
      -RedirectStandardOutput "$env:TEMP\skyland-chk.out" -RedirectStandardError "$env:TEMP\skyland-chk.err"
    Start-Sleep -Seconds 4
    foreach ($s in @('ws-tree-smoke.js','ux-smoke.js','agent-tree-smoke.js')) {
      $o = & node (Join-Path $scripts $s) 'http://127.0.0.1:3001' 2>&1 | Out-String
      $last = ($o.Trim() -split "`r?`n")[-1]
      if ($LASTEXITCODE -ne 0) { No "$s -> $last" } else { Ok "$last" }
    }
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Remove-Item Env:\SKYLAND_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:\SKYLAND_DESKTOP_MODE -ErrorAction SilentlyContinue
    Remove-Item $tmpData -Recurse -Force -ErrorAction SilentlyContinue
  }
}

Write-Host ''
if ($failed.Count -eq 0) { Write-Host 'ALL PASSED' -ForegroundColor Green; exit 0 }
Write-Host "$($failed.Count) FAILED" -ForegroundColor Red
exit 1