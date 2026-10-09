# Connect and test a data source

## Steps
1. Create a connection profile with host, port, database (or Oracle service),
   user; store the password in the OS credential store.
2. Run the connection test.
3. Repeat with a wrong password, an unknown database/service, a closed port.

## Expected
- Success reports the server version and the client/driver in use.
- Each failure names the cause (authentication, unknown database/service,
  host unreachable) and never prints the password.

## Source
https://www.jetbrains.com/help/pycharm/configuring-database-connections.html (Test Connection)
