# Main window areas while the window is resized

## Steps
1. Open a project and a file; show a tool window on the right (or drag the
   project tool window's splitter so the two side areas differ) and the
   Run tool window below the editor.
2. Make the window narrower step by step, down to its smallest width, then
   wider again.
3. Make the window lower step by step, down to its smallest height, then
   higher again.
4. Make the window narrow enough that the project tool window is squeezed,
   then drag its splitter.

## Expected
Across (continuous while resizing, no jumps, nothing overlaps):
- First only the editor gets narrower; the left and right areas keep their
  widths.
- With the editor at its minimum (30 px with the gaps around its island)
  both side areas get narrower together, each in proportion to its width,
  down to the same minimum. A side area alone does the same by itself.
- Below all three minimums the right area runs under the right stripe and
  is cut there. The content of an area is cut at its edge.
- Growing the window gives each width the same layout as on the way down
  and brings back the widths the areas had.

Down:
- First only the row of the editor and the side areas gets lower; the Run
  tool window keeps its height. With the row at its minimum the Run tool
  window gets lower, down to the same minimum. The toolbar and the status
  bar keep their heights.
- The stripes keep their top group at the top and their bottom group at the
  bottom; once the groups meet, the top group is drawn over the bottom one
  and covers as many pixels of it as the window loses (half a button can
  stay visible).
- Growing the window brings back the Run tool window's height.

Window: as narrow as 340 px; as low as the toolbar, the status bar and both
areas at their minimum.

Dragging a squeezed area's splitter starts from the width on screen; the
drag keeps the minimum and leaves the editor its minimum next to the other
side's width.

## Commands
`workbench.layout` (the sizes for the current window size),
`workbench.state` (`tool_window_areas`: the sizes the user gave),
`workbench.toggle-run`, `workbench.activate-tool-window {"id": ...}`.
`PYOXIDE_WINDOW_SIZE=<w>x<h>` opens the window at a given size.

## Source
observed (PyCharm 2026.2.3, four window widths and a very low window);
IntelliJ Platform Community at `pycharm/2026.2.3`: `ToolWindowPane.kt`
(splitters, `updateInnerMinSize`), `ThreeComponentsSplitter.kt` (`doLayout`,
`Divider.clamp`), `registry.properties` (`ide.mainSplitter.min.size=30`),
`IdeProjectFrameAllocator.kt` (frame minimum width 340),
`ToolWindowToolbar.kt` (stripe groups in a border layout).
