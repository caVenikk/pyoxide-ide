# Pyoxide shell integration for PowerShell (Windows PowerShell 5.1 and
# PowerShell 7). Pyoxide starts it as
# `pwsh -NoExit -ExecutionPolicy Bypass -File <this file>`, which runs after
# the user's profile. The file:
# - runs the scripts Pyoxide names (a virtual environment's `Activate.ps1`);
# - reports the working directory (OSC 7) and marks the prompt and the
#   commands with their exit codes (OSC 133 A, B, C, D) for the terminal.
# The prompt and PSReadLine hooks are adapted from the IntelliJ Community
# terminal plugin (Apache-2.0), see NOTICE.

# The scripts to run: __PYOXIDE_SOURCE_1, __PYOXIDE_SOURCE_2, ..., each
# "<script>`n<argument>`n...", dot-sourced.
$__PyoxideIndex = 1
while ($true) {
  $__PyoxideName = "__PYOXIDE_SOURCE_$__PyoxideIndex"
  $__PyoxideValue = [Environment]::GetEnvironmentVariable($__PyoxideName)
  if ([string]::IsNullOrEmpty($__PyoxideValue)) {
    break
  }
  Remove-Item -LiteralPath "Env:$__PyoxideName" -ErrorAction SilentlyContinue
  $__PyoxideParts = @($__PyoxideValue -split "`n")
  $__PyoxideArgs = @()
  if ($__PyoxideParts.Count -gt 1) {
    $__PyoxideArgs = @($__PyoxideParts[1..($__PyoxideParts.Count - 1)])
  }
  . $__PyoxideParts[0] @__PyoxideArgs
  $__PyoxideIndex++
}
Remove-Variable -Name __PyoxideIndex, __PyoxideName, __PyoxideValue, __PyoxideParts, __PyoxideArgs -ErrorAction SilentlyContinue

# The hooks need the full language (not in Constrained Language Mode).
if ($ExecutionContext.SessionState.LanguageMode -eq 'FullLanguage') {
  $Global:__PyoxideState = @{
    # A command runs (between C and D).
    Running = $false
    OriginalPrompt = $function:prompt
    OriginalReadLine = $null
  }

  function Global:__PyoxideOsc([string]$Body) {
    return "$([char]0x1B)]$Body$([char]0x07)"
  }

  # OSC 7: the working directory as a file URL ('%' and control characters
  # percent-encoded); only for folders of the file system.
  function Global:__PyoxideCwd {
    if ($PWD.Provider.Name -ne 'FileSystem') {
      return ''
    }
    $Path = $PWD.ProviderPath -replace '\\', '/'
    $Path = $Path -replace '%', '%25'
    $Path = [regex]::Replace($Path, '[\x00-\x1F\x7F]', { param($m) '%{0:X2}' -f [int][char]$m.Value })
    if (-not $Path.StartsWith('/')) {
      $Path = "/$Path"
    }
    return Global:__PyoxideOsc "7;file://$([Environment]::MachineName)$Path"
  }

  function Global:prompt {
    $Success = $Global:?
    $ExitCode = $Global:LASTEXITCODE
    $Result = ''
    if ($Global:__PyoxideState.Running) {
      $Global:__PyoxideState.Running = $false
      $Code = if ($Success) { 0 } elseif ($ExitCode -is [int] -and $ExitCode -ne 0) { $ExitCode } else { 1 }
      $Result += Global:__PyoxideOsc "133;D;$Code"
    }
    $Result += Global:__PyoxideCwd
    $Result += Global:__PyoxideOsc '133;A'
    # Restores $? for the original prompt; the error is not shown.
    if ($Success -eq $false) {
      Write-Error 'error' -ErrorAction Ignore
    }
    if ($null -ne $Global:__PyoxideState.OriginalPrompt) {
      $Result += & $Global:__PyoxideState.OriginalPrompt
    }
    else {
      $Result += "PS $($ExecutionContext.SessionState.Path.CurrentLocation)$('>' * ($NestedPromptLevel + 1)) "
    }
    $Result += Global:__PyoxideOsc '133;B'
    return $Result
  }

  # C: the line PSReadLine read is about to run (not an empty line).
  if ($null -ne (Get-Module -Name PSReadLine)) {
    $Global:__PyoxideState.OriginalReadLine = $function:PSConsoleHostReadLine
    function Global:PSConsoleHostReadLine {
      $Line = & $Global:__PyoxideState.OriginalReadLine
      if (-not [string]::IsNullOrWhiteSpace($Line)) {
        $Global:__PyoxideState.Running = $true
        [Console]::Write((Global:__PyoxideOsc '133;C'))
      }
      return $Line
    }
  }
}
