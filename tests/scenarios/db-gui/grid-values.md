# Value display in the grid

## Steps
1. Open a table with NULLs, empty strings, long text, json, arrays, timestamps with zone, numeric(30,10), bytea/BLOB and uuid.
2. Open the value panel on a long text cell and on a binary cell.
3. Switch binary display between text, hex, hex+ascii and uuid.

## Expected
- NULL looks different from an empty string.
- Exact numbers are shown without rounding (decimals stay exact beyond 64-bit).
- Very long values are cut at the fetch limit; the panel offers Load full value and Save to file.
- JSON, arrays and composite values are shown as text and open in the value panel.
- Date and number formats follow the settings.

## Source
ADR 0002 (typed values, Truncated); observed
