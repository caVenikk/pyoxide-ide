# The IDE shell integration for PowerShell (Windows PowerShell 5.1 and
# PowerShell 7). the IDE starts it as
# `pwsh -NoExit -ExecutionPolicy Bypass -File <this file>`, which runs after
# the user's profile. The file:
# - runs the scripts the IDE names (a virtual environment's `Activate.ps1`);
# - reports the working directory (OSC 7) and marks the prompt and the
#   commands with their exit codes (OSC 133 A, B, C, D) for the terminal.
# The prompt and PSReadLine hooks are adapted from the IntelliJ Community
# terminal plugin (Apache-2.0), see NOTICE.

# The scripts to run: __IDE_SOURCE_1, __IDE_SOURCE_2, ..., each
# "<script>`n<argument>`n...", dot-sourced.
$__IdeIndex = 1
while ($true) {
  $__IdeName = "__IDE_SOURCE_$__IdeIndex"
  $__IdeValue = [Environment]::GetEnvironmentVariable($__IdeName)
  if ([string]::IsNullOrEmpty($__IdeValue)) {
    break
  }
  Remove-Item -LiteralPath "Env:$__IdeName" -ErrorAction SilentlyContinue
  $__IdeParts = @($__IdeValue -split "`n")
  $__IdeArgs = @()
  if ($__IdeParts.Count -gt 1) {
    $__IdeArgs = @($__IdeParts[1..($__IdeParts.Count - 1)])
  }
  . $__IdeParts[0] @__IdeArgs
  $__IdeIndex++
}
Remove-Variable -Name __IdeIndex, __IdeName, __IdeValue, __IdeParts, __IdeArgs -ErrorAction SilentlyContinue

# The hooks need the full language (not in Constrained Language Mode).
if ($ExecutionContext.SessionState.LanguageMode -eq 'FullLanguage') {
  $Global:__IdeState = @{
    # A command runs (between C and D).
    Running = $false
    OriginalPrompt = $function:prompt
    OriginalReadLine = $null
  }

  function Global:__IdeOsc([string]$Body) {
    return "$([char]0x1B)]$Body$([char]0x07)"
  }

  # OSC 7: the working directory as a file URL ('%' and control characters
  # percent-encoded); only for folders of the file system.
  function Global:__IdeCwd {
    if ($PWD.Provider.Name -ne 'FileSystem') {
      return ''
    }
    $Path = $PWD.ProviderPath -replace '\\', '/'
    $Path = $Path -replace '%', '%25'
    $Path = [regex]::Replace($Path, '[\x00-\x1F\x7F]', { param($m) '%{0:X2}' -f [int][char]$m.Value })
    if (-not $Path.StartsWith('/')) {
      $Path = "/$Path"
    }
    return Global:__IdeOsc "7;file://$([Environment]::MachineName)$Path"
  }

  function Global:prompt {
    $Success = $Global:?
    $ExitCode = $Global:LASTEXITCODE
    $Result = ''
    if ($Global:__IdeState.Running) {
      $Global:__IdeState.Running = $false
      $Code = if ($Success) { 0 } elseif ($ExitCode -is [int] -and $ExitCode -ne 0) { $ExitCode } else { 1 }
      $Result += Global:__IdeOsc "133;D;$Code"
    }
    $Result += Global:__IdeCwd
    $Result += Global:__IdeOsc '133;A'
    # Restores $? for the original prompt; the error is not shown.
    if ($Success -eq $false) {
      Write-Error 'error' -ErrorAction Ignore
    }
    if ($null -ne $Global:__IdeState.OriginalPrompt) {
      $Result += & $Global:__IdeState.OriginalPrompt
    }
    else {
      $Result += "PS $($ExecutionContext.SessionState.Path.CurrentLocation)$('>' * ($NestedPromptLevel + 1)) "
    }
    $Result += Global:__IdeOsc '133;B'
    return $Result
  }

  # C: the line PSReadLine read is about to run (not an empty line).
  if ($null -ne (Get-Module -Name PSReadLine)) {
    $Global:__IdeState.OriginalReadLine = $function:PSConsoleHostReadLine
    function Global:PSConsoleHostReadLine {
      $Line = & $Global:__IdeState.OriginalReadLine
      if (-not [string]::IsNullOrWhiteSpace($Line)) {
        $Global:__IdeState.Running = $true
        [Console]::Write((Global:__IdeOsc '133;C'))
      }
      return $Line
    }
  }
}
