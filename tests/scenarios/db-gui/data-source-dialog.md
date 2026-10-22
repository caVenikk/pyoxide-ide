# Add a data source and test the connection

## Steps
1. Open the Database tool window with no data source: an empty state offers to create one.
2. New > Data Source, pick PostgreSQL. Fill host, port, database, user and password; the URL field shows the same values and editing it updates the fields.
3. Press Test Connection with a wrong password, then with the right one.
4. Save with the password mode "Do not save"; disconnect and connect again.
5. Repeat with an Oracle service name and with a SQLite file (a path, no host).

## Expected
- A wrong password reports the server's reason and a hint; nothing is saved by the test.
- A good test shows the server and driver versions.
- The password is never written to a file or a log; with "Do not save" it is asked on every connect.
- A SQLite data source has only a file path; a missing file is reported, not created silently.
- The data source survives a restart with all options; deleting it deletes its saved secrets.

## Source
https://www.jetbrains.com/help/pycharm/data-sources-and-drivers-dialog.html, https://www.jetbrains.com/help/pycharm/configuring-database-connections.html; observed
