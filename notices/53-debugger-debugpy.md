- The debugger helper: debugpy 1.8.21 (MIT, (c) Microsoft Corporation),
  https://github.com/microsoft/debugpy @ v1.8.21, the PyPI wheel
  debugpy-1.8.21-py2.py3-none-any.whl (SHA-256
  b1e37d333663c8851516a47364ef473da127f9caebe4417e6df6f5825a7e9a92).
  Not in this repository: scripts/fetch-debugpy.py checks the wheel and
  unpacks it unmodified into the application cache (development builds)
  or the release bundle's helpers/debugpy (packaging); its files are never
  patched. debugpy vendors PyDev.Debugger (pydevd,
  https://github.com/fabioz/PyDev.Debugger) under the Eclipse Public
  License 1.0, and other parts (CPython, IPython, six, WinAppDbg, ...)
  under their licences, all listed with their texts, the EPL-1.0 text
  included, in debugpy/ThirdPartyNotices.txt of the wheel. The wheel's
  compiled parts (pydevd_attach_to_process: attach*.dll, .so, .dylib,
  inject_dll*.exe, run_code_on_dllmain*.dll and their .pdb; the Windows
  .pyd of pydevd_cython and pydevd_frame_evaluator) come with their sources
  (.cpp, .c, .pyx) in the same folder. Pyoxide starts debugpy as a separate
  process and links none of it.
  Release packaging ships the folder as unpacked, keeping
  debugpy/ThirdPartyNotices.txt, debugpy-1.8.21.dist-info/licenses/LICENSE
  and every notice in the files, and its third-party notices name debugpy
  (MIT) and pydevd (EPL-1.0: the files under debugpy/_vendored/pydevd are
  its unmodified source, from https://github.com/microsoft/debugpy/tree/
  v1.8.21/src/debugpy/_vendored/pydevd) with a pointer to
  ThirdPartyNotices.txt for the licence texts.
