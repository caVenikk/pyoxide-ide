- The ConPTY runtime of Windows release archives: conpty.dll and OpenConsole.exe
  from the NuGet package Microsoft.Windows.Console.ConPTY 1.24.261001001
  (MIT, (c) Microsoft Corporation), built from Windows Terminal,
  https://github.com/microsoft/terminal (package page:
  https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY/1.24.261001001;
  SHA-512 263e7cb79524c99b4893feafa9c97744fb6d3104a3847243acea306c2976ee89
  f98d0bdda0775abfe94a5b015263bfad56bb1565fc9442c4f50a94e02bed54f2).
  Not in this repository: scripts/fetch-conpty.py checks the package and
  scripts/package.py places the two x64 files unmodified next to the
  executable, where alacritty_terminal picks them up (ADR 0014). Pyoxide
  starts them as part of the pseudo console and links none of their code.
