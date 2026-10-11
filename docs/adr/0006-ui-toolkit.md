# 0006. UI toolkit

- Status: accepted
- Date: 2026-10-11

## Context
The IDE needs one Rust UI toolkit for every window: the editor, tool windows,
dialogs and the database grid. Requirements, in order of weight:

1. **Visual fidelity.** The UI follows the reference IDE's current look
   closely, down to small details (docs/clean-room.md, "Visual fidelity"):
   exact metrics, rounded panels, per-side borders, hairlines on 2x
   displays, custom fonts at exact sizes and weights with comparable
   antialiasing, crisp SVG icons, focus rings, hover states, popups with
   shadows, the title bar area used by the toolbar, themes mapped from
   imported colour schemes. Every widget is custom-painted; no fixed native
   or library styles.
2. **Performance** within docs/performance.md: start to an interactive
   editor <= 0.3 s on macOS, keystroke to paint p99 <= 16 ms, idle memory
   <= 300 MB, nothing blocks the UI thread. The most frequent complaint about
   the reference database tools is the grid freezing on large cells
   (replica/feedback.md): the grid must stay smooth with 100,000 rows and
   10 KB values.
3. **Editor fit.** A custom code editor (millions of lines, custom text
   painting, gutters, inlays, IME), docking tool windows, several windows,
   keymaps with context-dependent bindings.
4. **Headless testability.** Agents drive the IDE: tests find widgets, inject
   input and, ideally, render images without a window. Every user action is
   also a command in the registry (ADR 0001), so agents do not depend on the
   widget tree for control.
5. **Platforms and accessibility.** Windows, macOS and Linux with a GPU
   renderer; screen reader support.
6. **License** compatible with distributing an MIT OR Apache-2.0
   application, including forks and a future plugin API (M7).

## Options considered
Four toolkits were built into the same small throwaway prototype (not kept
in the repository; the numbers below are its results): a database browser (profiles, SQL box, a virtualized
grid fed page by page from `Cursor::next_page`, all database work on a
background thread, cancel) and a static mock of an IDE window chrome with
exact metrics, fonts (Inter, JetBrains Mono) and SVG icons. One scripted
benchmark ran inside every prototype and the same scripts measured them.
Floem and Xilem were not built: both are pre-1.0 with small ecosystems and no
editor-scale application besides their authors' own.

| | egui + eframe | iced | Slint | GPUI |
|---|---|---|---|---|
| version | 0.36.2 (wgpu 30, winit 0.30) | 0.14.0 (wgpu 27, winit 0.30) | 1.18.1 (winit 0.30) | 0.2.2 from crates.io = Zed commit 69e2130 (Oct 2025) |
| model | immediate mode, explicit painter | Elm architecture, view rebuilt after every message | `.slint` markup compiled to Rust, reactive properties | entities + element tree rebuilt per frame, flexbox layout (taffy), actions and key contexts |
| renderer on macOS | wgpu / Metal | wgpu / Metal (tiny-skia fallback) | FemtoVG / OpenGL; optional Skia via wgpu / Metal | own Metal renderer |
| other platforms | wgpu (Vulkan, DX12, GL) or glow | wgpu or tiny-skia | FemtoVG, Skia, software; LinuxKMS, Qt | Windows: Direct3D 11 + DirectWrite; Linux: Vulkan (blade) on Wayland and X11 |
| text | harfrust shaping, skrifa outlines, vello_cpu raster, grayscale, optional hinting | cosmic-text (harfrust), swash raster, grayscale, hinted | parley / harfrust / fontique; FemtoVG atlas or Skia, grayscale, unhinted | Core Text shaping and rasterization on macOS (as native apps), DirectWrite on Windows, cosmic-text on Linux |
| license | MIT OR Apache-2.0 | MIT | GPL-3.0-only OR Slint Royalty-free 2.0 OR Slint Software 3.0 (paid) | Apache-2.0 |

### Measurements
Machine: Apple M5 (10 cores), 16 GB, macOS 27.0.1, built-in 60 Hz display at
2x, Rust 1.99.0, release builds with the default profile, windows in front
and uncovered (checked at every marker), each toolkit started at a load
average below 4. Raw data is attached to issue #12.

| metric | egui | iced | Slint (FemtoVG) | Slint (Skia) | GPUI |
|---|---|---|---|---|---|
| first frame, warm, median / worst (5 runs) | 131 / 134 ms | 120 / 121 ms | 140 / 143 ms | 168 / 171 ms | **84 / 86 ms** |
| first frame, cold-ish, median / worst (5 runs) | 521 / 791 ms | 496 / 1066 ms | 485 / 814 ms | 578 / 820 ms | **405** / 964 ms |
| typing into the SQL box, p50 / p99 (3 x 200 keys) | 22.0 / 32.7 ms | **7.7 / 9.9 ms** | 6.3 / 17.3 ms | 6.3 / 17.3 ms | 7.7 / 19.2 ms |
| Run to first rows painted | 33 ms | 33 ms | 35 ms | 24 ms | 31 ms |
| scrolling 100k rows in 5 s: frame interval p50 / p99 | 16.7 / 17.0 ms | 16.7 / 17.2 ms | 16.7 / 17.4 ms | 16.7 / 18.2 ms | 16.7 / 17.9 ms |
| frames over 25 ms while scrolling | 0 | 0 | 0 | 0 | 0 |
| UI-thread work per scrolled frame, p50 / p99 | 2.1 / 2.8 ms | 3.8 / 8.3 ms | 1.4 / 2.4 ms | 2.7 / 3.5 ms | 6.6 / 7.7 ms |
| memory with 100k rows loaded, RSS / footprint | 150 / 186 MB | 134 / 174 MB | 139 / 214 MB | 149 / 170 MB | 137 / 158 MB |
| memory after 30 s idle, RSS / footprint | 118 / 184 MB | 98 / 170 MB | 124 / 212 MB | 149 / 165 MB | 130 / **112** MB |
| CPU while idle (caret blinking) | 2.1 % | 0.6 % | 0.9 % | 1.0 % | 0.5 % |
| clean release build | 63 s | 63 s | 76 s | 97 s | 91 s |
| binary, as built / stripped | 26.1 / 21.6 MB | 26.3 / 21.3 MB | 28.4 / 23.6 MB | 38.1 / 31.0 MB | 22.5 / 18.6 MB |
| crates in the build graph (macOS) | 339 | 348 | 417 | 459 | 562 |

The shared part (database worker, the four drivers, result model) alone
builds in 32 s with 209 crates, so the toolkits add 130 (egui) to 353 (GPUI)
crates; every binary includes the drivers (TLS, SQLite, Oracle clients).

How to read the table:
- "First frame" is wall time from spawning the process to the end of the
  first render pass. Cold-ish launches a fresh copy of the binary (dyld,
  signature and shader caches cold, file cache warm); 280 to 350 ms of it
  pass before `main`, about the same for every toolkit. GPUI compiles its
  Metal shaders at start-up here (`runtime_shaders`, no Xcode on the
  machine); precompiled shaders would make it faster still.
- Typing latency runs from the moment a key is due to the end of the render
  pass of the first frame that shows it, with keys injected through each
  toolkit's own input pipeline (egui raw input, Slint `dispatch_event`, GPUI
  `dispatch_keystroke`; iced has no public injection API, so keys enter the
  widget tree from a root wrapper widget). It is not photon latency. In
  egui the frame starts at the next display refresh (0 to 16 ms after the
  key), builds its UI in 0.3 ms and then waits one full refresh before its
  render pass (surface acquisition in eframe/wgpu on Metal; the same with a
  frame latency of 2), so its number contains
  one refresh more than the others. The p99 of Slint and GPUI is one refresh
  plus a few milliseconds: they render on the display link.
- "Work per frame" is measured between different points in each toolkit
  (egui: `ui()`; iced: update to draw; Slint: before to after rendering;
  GPUI: render start to the end of paint, including layout and text shaping
  of the rows scrolled in) and is comparable only roughly. Every toolkit kept
  the 60 Hz frame rate with 333 new rows per frame and 10 KB cells.

### Visual fidelity
Every prototype drew the same mock; a script probed the 2560x1600
screenshots pixel by pixel, and side-by-side renderings of the same text
compared glyph rendering (egui, iced, Slint FemtoVG, Slint Skia, GPUI).

| | egui | iced | Slint | GPUI |
|---|---|---|---|---|
| geometry (islands, focus ring 2 px outside a 1 px border, 2 px tab underline, selection insets) | exact | exact | exact | exact |
| 1 physical px hairline at 2x | exact | exact | exact (`1phx`) | exact |
| per-corner radius | yes (integer radii) | yes | yes | yes |
| per-side borders | painted by hand | no (nested containers) | no (child rectangles) | **yes** (`border_b`, ...) |
| drop shadow | yes (blur, spread) | yes (no spread) | yes | yes |
| text baseline at the specified position | **exact** (glyph positions) | within 0.5 px (no baseline API) | exact (`font-metrics`) | exact (computed line height) |
| letter spacing | yes | no | yes | no |
| OpenType features (e.g. ligatures off) | no (ligatures always on) | basic vs advanced shaping only | no (ligatures always on) | **yes** (`FontFeatures`) |
| fonts from bytes, weights 400/500/600 | yes (one family per weight; variable fonts) | yes | yes | yes |
| glyph rendering on macOS | own rasterizer, lighter than native | own rasterizer, lighter than native | FemtoVG lighter; Skia close to native | **Core Text, as native apps** |
| colour SVG crisp at 2x | yes (resvg at device size) | yes | yes | monochrome `svg()` only; colour SVG via `img()` is rasterized at 1x |
| title bar: content under it, traffic lights placed | content yes, lights fixed | content yes, lights fixed | content yes, lights fixed | **yes, lights positioned** |
| mock size (lines) | 451 | 629 | 855 | 577 |

All four reproduce our metrics to the physical pixel; no toolkit forced a
native style or blocked a primitive. The visible differences are in text:
egui, iced and FemtoVG rasterize glyphs themselves and look lighter than the
system's text, while GPUI uses Core Text, the system rasterizer that native
macOS applications use, and Skia comes close to it. On Windows GPUI uses
DirectWrite; the others look the same on every platform. Missing pieces are
small and fillable in our own components:
per-side borders (iced, Slint), baseline arithmetic (GPUI), letter spacing
(iced, GPUI), rounded clipping (GPUI clips to rectangles), crisp colour SVG
in GPUI (rasterize with resvg at device scale ourselves).

### Headless testability
Each spike has a test that types a query into the SQL box, presses Run and
waits for "3 rows" without a window; all passed.

| | egui (egui_kittest) | iced (iced_test) | Slint (testing backend) | GPUI (TestAppContext) |
|---|---|---|---|---|
| find widgets | AccessKit role and label | widget id, text, closure | accessible label (needs compiler debug info) | debug selectors (test builds only) |
| inject input | yes, egui events | yes, simulator events | yes, `dispatch_event` | yes, `simulate_keystrokes` / `simulate_input` |
| image without a window | yes, wgpu offscreen, same renderer as on screen | yes, wgpu or tiny-skia, bit-identical to the window | software renderer only (differs from FemtoVG / Skia) | no (test platform does not render) |
| screenshot of the running window | yes (viewport screenshot) | yes (`window::screenshot`) | yes (`take_snapshot`) | no API; a capture of the app's own window through the window server works without the Screen Recording permission |
| API status | public, versioned with egui | public | internal crate, pin `=x.y.z` | public with `test-support` |

### Accessibility, IME, platforms
| | egui | iced | Slint | GPUI |
|---|---|---|---|---|
| screen readers | AccessKit (macOS, Windows, Linux) | none | AccessKit | none in 0.2.2 |
| IME | winit IME, preedit in `TextEdit` | winit IME, preedit in `text_editor` | winit IME, preedit in `TextInput` | `NSTextInputClient` through the input handler (the path Zed uses) |
| open without stealing focus | yes (winit option) | no (event loop built inside iced) | yes (winit hooks) | yes (`focus: false`) |
| Windows / Linux | yes / yes | yes / yes | yes / yes | yes (Direct3D 11) / yes (Vulkan); younger ports |

None of this was tested on Windows or Linux here.

### Licenses
- **egui, eframe, egui_extras, egui-wgpu**: MIT OR Apache-2.0. The default
  fonts crate bundles fonts under the Ubuntu Font Licence and OFL; the IDE
  would disable `default_fonts` and ship its own fonts.
- **iced**: MIT.
- **GPUI**: Apache-2.0, as are its sub-crates published with it
  (`gpui_collections`, `gpui_util`, `gpui_sum_tree`, `gpui_refineable`,
  `gpui_media`, `gpui_http_client`, ...) and the Zed forks it pulls in
  (`zed-font-kit`, `zed-reqwest`: MIT OR Apache-2.0). Zed's application
  crates (the editor, workspace and UI components among them) are not part
  of GPUI and are GPL-licensed; they must not be used or copied. The
  third-party `gpui-component` library is Apache-2.0. In the GPUI build
  graph the only non-permissive item is `cbindgen` (MPL-2.0), a build-time
  tool that is not linked.
- **Slint** offers three licenses (`GPL-3.0-only OR
  LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`, texts
  in the crate's `LICENSES/`):
  - GPL-3.0-only: our MIT and Apache-2.0 code may be combined with it, but
    every distributed binary is then covered by the GPL-3.0, so nobody can
    ship a closed-source fork or reuse the platform layer in a proprietary
    product without a commercial Slint license.
  - Royalty-free 2.0: free for desktop applications if the About dialog
    shows the `AboutSlint` widget or the download page shows the "Made with
    Slint" badge. It forbids use in embedded systems, distribution of Slint
    on its own, and "the distribution of Application that exposes the APIs,
    in part or in total, of the Software", which conflicts with a plugin API
    that lets extensions build UI. It is not an open-source license, so every
    redistributor (forks, Linux distributions) must accept it or fall back
    to the GPL.
  - Slint Software 3.0: paid per developer seat, with audit rights.
  Slint would be the only non-permissive component of the IDE.

The full list of non-permissive licenses in each build graph (fonts, MPL,
Unicode data) is part of the raw data attached to issue #12.

## Decision
GPUI, pinned to a Zed commit, is the UI toolkit; egui is the runner-up and
the fallback.

Reasons:
- Visual fidelity: it reproduced every metric of the mock, has per-side
  borders and title-bar control built in, its flexbox styles map one to one
  onto design metrics, and on macOS its text is rasterized by Core Text, so
  glyphs look like those of native applications. egui and iced matched the
  geometry as exactly but render visibly lighter text.
- Performance: the fastest start (84 ms warm), the smallest idle footprint
  and idle CPU, typing and scrolling within budget.
- Editor fit: the same element, text-layout, list, input-handler (IME) and
  action/key-context machinery carries Zed, a full code editor; key contexts
  match how the reference keymaps bind actions per component.
- License: Apache-2.0.

The price is the weakest testing and accessibility story of the four, an
unreleased upstream and the highest UI-thread work per scrolled frame
(6.6 ms, still well inside a 60 Hz frame); see Consequences.

Why not the others:
- egui: the best testing and accessibility story (AccessKit, offscreen
  rendering with the real renderer) and stable releases, but lighter
  non-native text, an immediate-mode layout that needs extra passes for
  content-sized layouts, and one extra refresh of latency before rendering
  in eframe on macOS. It stays the fallback if GPUI's upstream becomes
  unusable.
- iced: matched the mock, but lacks per-side borders, letter spacing,
  baseline control, accessibility and input injection, and rebuilds the
  whole view after every message.
- Slint: its license does not fit an MIT OR Apache-2.0 project with plugins,
  and it has no immediate drawing API for a custom code editor.

## Consequences
- GPUI is not released for third parties: the crates.io release (0.2.2) is a
  year behind Zed, APIs change without notice and documentation is the
  examples and the source. We pin a Zed commit (or the weekly `gpui-pre`
  snapshot published by the `gpui-component` maintainers, Apache-2.0),
  upgrade deliberately, and keep toolkit code behind our own component layer
  in the platform crates, so a later move (to a newer GPUI or to egui) stays
  contained.
- We write our own components (inputs, trees, tabs, splitters, popups,
  docking) on GPUI primitives; Zed's GPL crates are off limits, and
  `gpui-component` is evaluated as a source of Apache-2.0 components with a
  NOTICE entry for anything adapted.
- No accessibility layer exists in GPUI: building one (AccessKit on our
  components, or upstream work) is a planned work item and a known gap
  until then.
- Testing: agents control the IDE through the command registry; UI tests use
  `TestAppContext` with our own element ids; visual checks capture the
  app's own window on a macOS runner with a display (no permission needed;
  the Core Graphics call used is deprecated, ScreenCaptureKit is the
  replacement). Headless image rendering would need an offscreen path in
  the renderer.
- Build: without Xcode the `runtime_shaders` feature compiles Metal shaders
  at start-up; release builds precompile them (Xcode on the build machine).
- Colour SVG icons must be rasterized at device scale by our icon loader
  (GPUI's `img()` rasterizes SVG at 1x); monochrome icons use `svg()`.
- The UI layer reads the roles, metrics and fonts of the `themes` crate
  (docs/theme-mapping.md) the way the mock read its tokens. Underline styles
  beyond plain and wavy (dotted, dashed, double) are painted by our editor
  element; the ligature setting maps to `FontFeatures`.
- Not yet known: Windows and Linux numbers (same scripts; GPUI's Windows
  port is the youngest part), IME with a real input method, bidi caret
  movement, and how much faster start-up is with precompiled shaders.
- The open-source IntelliJ Platform Community repository publishes its icon
  set and default themes under Apache-2.0; they may be used later with a
  NOTICE entry (source, commit, files). Nothing is copied from it now.
