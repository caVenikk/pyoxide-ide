# SQL dialect and resolution scope

## Steps
1. Open a `.sql` file in a project with no mapping; look at the dialect widget.
2. Map the folder `migrations/` to PostgreSQL and to a data source and schema; open a file in it.
3. Map a file inside it to Oracle.
4. Open a console of an SQLite data source.
5. Type `select 1 from dual` in the PostgreSQL file and in the Oracle one; type `select * from t returning id` in the generic one.

## Expected
- Step 1: the widget shows the project default (generic when none); step 2: PostgreSQL and the data source; the closest mapping wins in step 3.
- Step 4: the console is SQLite whatever the mappings say.
- Step 5: `dual` resolves in Oracle only; constructs of another dialect are not reported as errors in generic, and are weak warnings when the dialect is known.
- Mappings survive a restart; shared ones are in the project settings, personal ones are not.

## Source
Public documentation of SQL dialects and SQL resolution scopes.
