# Connection diagnostics (own feature)

## Steps
Run `pyoxide-db diagnose --profile <name>` against: a healthy database; a
wrong password; an unknown service; a closed port; an unresolvable host; a
server that requires TLS or native network encryption.

## Expected
- One line per step (profile, secret source, DNS, TCP, TLS, login, server
  version, dictionary access, typed round trip) with OK/FAIL and timing.
- The first failure has a plain-language cause and a next step.
- No password, wallet password or secret appears in the output; `--share`
  also masks host names, addresses, user and service names.

## Source
Own feature; error codes from the Oracle Database Error Messages and the
PostgreSQL error code appendix.
