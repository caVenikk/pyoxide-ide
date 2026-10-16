# Notification balloons

## Steps
1. Open a project. Run `workbench.notify` with kind `information`, a title,
   a content text and one action link (`workbench.state` as its command).
   Leave the mouse outside the window and do not touch the keyboard.
2. Move the mouse over the window (not over the balloon) and wait ten
   seconds.
3. Run `workbench.notify` again; move the mouse over the balloon and wait
   longer than ten seconds, then move it away and wait ten seconds.
4. Run `workbench.notify` with kind `error`; wait ten seconds, then close
   it with the x button that appears under the mouse.
5. Run `workbench.notify` four times (kinds information, warning, error,
   information, with different titles). Click the "1 more notification"
   line.
6. Click the action link of a balloon.
7. Switch to another application while a balloon is shown and come back
   after more than ten seconds.
8. Open a project with run configurations of the reference IDE that are not
   imported yet (see `run-debug/run-configurations.md`), then run
   `run.configurations.import-reference` again.
9. Switch between the dark and light theme with a balloon shown.

## Expected
- The balloon sits at the bottom right of the main window: its right edge
  10 px left of the right stripe, its bottom edge 10 px above the status
  bar, over the tool windows and the editor and under popups and dialogs.
  It has a fixed width; on the left the kind's icon (information, warning or
  error), then the title in bold (only when there is content; a balloon
  with only a title shows it in the normal weight), the content below it
  (wrapped at the balloon's width) and the action links in a row below that,
  16 px apart, underlined under the pointer.
- Step 1: an information balloon is not removed before the first input
  after it was shown; the close button is not shown.
- Step 2: the balloon goes ten seconds after the first input.
- Step 3: while the pointer is over the balloon it stays and shows its
  close button (top right, a rounded background under the pointer); after
  the pointer leaves the time runs again, from what was left.
- Step 4: an error balloon stays (five minutes) until it is closed; the
  button closes it at once (with Alt held, every balloon closes).
- Step 5: at most three balloons are shown, the newest on top; the oldest
  collapses into a line "1 more notification" under the stack ("2 more
  notifications" for two). A click on the line closes every balloon (the
  reference opens the Notifications tool window, which does not exist yet).
- Step 6: the balloon closes and the link's command runs.
- Step 7: the time does not run while the window is inactive; the balloon
  is still there on return.
- Step 8: the import report ("Imported 2 run configurations") is a
  balloon, not text in the status bar; the second import shows "No new run
  configurations to import". A configuration file that cannot be read or
  saved gives an error balloon.
- Step 9: the balloon's colours follow the theme (background, border,
  title, content, link, shadow).
- `workbench.state` (`notifications`) lists the open balloons, the newest
  first: id, kind, title, content, action texts, sticky, hovered, whether
  the time runs and the milliseconds left.

## Commands
`workbench.notify {"kind": "information"|"warning"|"error", "title": ...,
"content": ..., "sticky": ..., "actions": [{"text": ..., "command": ...,
"args": ...}]}`, `workbench.state` (`notifications`),
`workbench.set-theme {"id": ...}`.

## Notes
- The reference also lists every notification in the Notifications tool
  window and has per-group "Don't show again" settings; neither exists yet
  (the stripe button has no tool window behind it).

## Source
https://www.jetbrains.com/help/pycharm/notifications.html and the sources
of the reference (Apache-2.0, tag `pycharm/2026.2.3`).
