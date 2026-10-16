# Change, Revert, Apply, OK and Cancel

## Steps
1. Open Appearance & Behavior > System Settings; change "Save files if
   the IDE is idle for" from 15 to 30 seconds.
2. Look at the tree, the banner and the bottom bar.
3. Click "Revert changes".
4. Change the value to 30 again and press Apply.
5. Change a check box on the page and press Cancel; reopen Settings.
6. Change the value, then close the project.
7. Enter an invalid value and press Apply.

## Expected
- Step 2: the page's name in the tree takes the modified colour; "Revert
  changes" appears in the banner; Apply is enabled.
- Step 3: the stored value 15 is back; the page is no longer modified.
- Step 4: the settings file holds the value; autosave waits 30 seconds of
  idle time; Apply is disabled again.
- Step 5: the check box is unchanged after reopening.
- Step 6: the message "Unsaved Changes" with Apply, Don't Save and
  Cancel.
- Step 7: the page stays selected, its name turns red and the error is
  shown under the page.

## Commands
`settings.select {"page": ...}`, `settings.set {"key": ..., "value":
...}`, `settings.revert`, `settings.apply`, `settings.ok`,
`settings.cancel`, `settings.state`.

## Source
https://www.jetbrains.com/help/pycharm/settings-preferences-dialog.html,
https://www.jetbrains.com/help/pycharm/system-settings.html; IntelliJ
Community (Apache-2.0) at `pycharm/2026.2.3`: `SettingsEditor`,
`GeneralSettingsConfigurable`.
