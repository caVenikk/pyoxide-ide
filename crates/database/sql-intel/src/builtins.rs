//! What a dialect has by itself: built-in functions with their parameters,
//! data types and keywords with one line of description each. The texts are
//! our own short wording; they feed completion, quick documentation and
//! parameter info.

use sql_syntax::Dialect;

/// How a function is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FnKind {
    Scalar,
    Aggregate,
    Window,
    /// Written without parentheses (`current_date`).
    Special,
}

/// A built-in function (one overload).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Function {
    pub name: &'static str,
    /// The parameters, comma separated; an optional one in brackets
    /// (`[digits]`), a variable tail as `...`.
    pub params: &'static str,
    pub returns: &'static str,
    pub doc: &'static str,
    pub kind: FnKind,
}

/// One parameter of a built-in function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Param {
    pub name: String,
    pub optional: bool,
}

impl Function {
    pub fn parameters(&self) -> Vec<Param> {
        self.params
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(
                |p| match p.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                    Some(inner) => Param {
                        name: inner.trim().to_string(),
                        optional: true,
                    },
                    None => Param {
                        name: p.to_string(),
                        optional: false,
                    },
                },
            )
            .collect()
    }

    /// `lower(string) -> text`, with optional parameters in brackets.
    pub fn signature(&self) -> String {
        let mut s = String::from(self.name);
        if self.kind != FnKind::Special {
            s.push('(');
            s.push_str(self.params);
            s.push(')');
        }
        if !self.returns.is_empty() {
            s.push_str(" -> ");
            s.push_str(self.returns);
        }
        s
    }
}

const fn f(
    name: &'static str,
    params: &'static str,
    returns: &'static str,
    doc: &'static str,
) -> Function {
    Function {
        name,
        params,
        returns,
        doc,
        kind: FnKind::Scalar,
    }
}

const fn agg(
    name: &'static str,
    params: &'static str,
    returns: &'static str,
    doc: &'static str,
) -> Function {
    Function {
        name,
        params,
        returns,
        doc,
        kind: FnKind::Aggregate,
    }
}

const fn win(
    name: &'static str,
    params: &'static str,
    returns: &'static str,
    doc: &'static str,
) -> Function {
    Function {
        name,
        params,
        returns,
        doc,
        kind: FnKind::Window,
    }
}

const fn sp(name: &'static str, returns: &'static str, doc: &'static str) -> Function {
    Function {
        name,
        params: "",
        returns,
        doc,
        kind: FnKind::Special,
    }
}

/// Functions every dialect here has.
const COMMON: &[Function] = &[
    agg(
        "count",
        "expression",
        "bigint",
        "Counts the rows of the group, or the rows where the expression is not null. count(*) counts all rows.",
    ),
    agg(
        "sum",
        "expression",
        "number",
        "Adds up the non-null values of the group.",
    ),
    agg(
        "avg",
        "expression",
        "number",
        "The average of the non-null values of the group.",
    ),
    agg("min", "expression", "", "The smallest value of the group."),
    agg("max", "expression", "", "The largest value of the group."),
    f(
        "coalesce",
        "value, ...",
        "",
        "Returns the first argument that is not null.",
    ),
    f(
        "nullif",
        "value, other",
        "",
        "Returns null when the two values are equal, otherwise the first value.",
    ),
    f("abs", "number", "number", "The absolute value of a number."),
    f(
        "round",
        "number, [digits]",
        "number",
        "Rounds a number to the given count of decimal places (none by default).",
    ),
    f(
        "sign",
        "number",
        "integer",
        "-1, 0 or 1 by the sign of the number.",
    ),
    f(
        "length",
        "string",
        "integer",
        "The number of characters of a string.",
    ),
    f(
        "lower",
        "string",
        "text",
        "Converts a string to lower case.",
    ),
    f(
        "upper",
        "string",
        "text",
        "Converts a string to upper case.",
    ),
    f(
        "replace",
        "string, search, replacement",
        "text",
        "Replaces every occurrence of a substring.",
    ),
    f(
        "ltrim",
        "string, [characters]",
        "text",
        "Removes characters (spaces by default) from the start of a string.",
    ),
    f(
        "rtrim",
        "string, [characters]",
        "text",
        "Removes characters (spaces by default) from the end of a string.",
    ),
    sp("current_date", "date", "The current date."),
    sp(
        "current_timestamp",
        "timestamp",
        "The current date and time.",
    ),
];

const POSTGRES: &[Function] = &[
    agg(
        "string_agg",
        "expression, delimiter",
        "text",
        "Joins the values of the group into one string with a delimiter.",
    ),
    agg(
        "array_agg",
        "expression",
        "array",
        "Collects the values of the group into an array.",
    ),
    agg(
        "bool_and",
        "condition",
        "boolean",
        "True when the condition holds for every row of the group.",
    ),
    agg(
        "bool_or",
        "condition",
        "boolean",
        "True when the condition holds for at least one row of the group.",
    ),
    agg(
        "json_agg",
        "expression",
        "json",
        "Collects the values of the group into a JSON array.",
    ),
    agg(
        "jsonb_agg",
        "expression",
        "jsonb",
        "Collects the values of the group into a JSONB array.",
    ),
    agg(
        "jsonb_object_agg",
        "key, value",
        "jsonb",
        "Builds a JSONB object from key and value pairs of the group.",
    ),
    agg(
        "percentile_cont",
        "fraction",
        "double precision",
        "The continuous percentile; used with WITHIN GROUP (ORDER BY ...).",
    ),
    win(
        "row_number",
        "",
        "bigint",
        "The number of the row within its partition, starting at 1.",
    ),
    win(
        "rank",
        "",
        "bigint",
        "The rank of the row with gaps after ties.",
    ),
    win(
        "dense_rank",
        "",
        "bigint",
        "The rank of the row without gaps after ties.",
    ),
    win(
        "ntile",
        "buckets",
        "integer",
        "Splits the partition into the given number of groups and returns the group of the row.",
    ),
    win(
        "lag",
        "value, [offset], [default]",
        "",
        "The value from an earlier row of the partition.",
    ),
    win(
        "lead",
        "value, [offset], [default]",
        "",
        "The value from a later row of the partition.",
    ),
    win(
        "first_value",
        "value",
        "",
        "The value from the first row of the window frame.",
    ),
    win(
        "last_value",
        "value",
        "",
        "The value from the last row of the window frame.",
    ),
    f(
        "char_length",
        "string",
        "integer",
        "The number of characters of a string.",
    ),
    f(
        "concat",
        "value, ...",
        "text",
        "Joins the values as text; nulls are skipped.",
    ),
    f(
        "concat_ws",
        "separator, value, ...",
        "text",
        "Joins the values with a separator; nulls are skipped.",
    ),
    f(
        "left",
        "string, count",
        "text",
        "The first characters of a string.",
    ),
    f(
        "right",
        "string, count",
        "text",
        "The last characters of a string.",
    ),
    f(
        "lpad",
        "string, length, [fill]",
        "text",
        "Pads a string on the left to the given length.",
    ),
    f(
        "rpad",
        "string, length, [fill]",
        "text",
        "Pads a string on the right to the given length.",
    ),
    f(
        "btrim",
        "string, [characters]",
        "text",
        "Removes characters (spaces by default) from both ends of a string.",
    ),
    f(
        "initcap",
        "string",
        "text",
        "Capitalizes the first letter of each word.",
    ),
    f(
        "substring",
        "string, start, [count]",
        "text",
        "A part of a string from a position, with an optional length.",
    ),
    f(
        "position",
        "substring IN string",
        "integer",
        "The position of a substring in a string, 0 when absent.",
    ),
    f(
        "split_part",
        "string, delimiter, index",
        "text",
        "The part of a string between delimiters, counted from 1.",
    ),
    f(
        "regexp_replace",
        "string, pattern, replacement, [flags]",
        "text",
        "Replaces the part of a string that matches a regular expression.",
    ),
    f(
        "regexp_match",
        "string, pattern, [flags]",
        "text[]",
        "The captured parts of the first regular expression match.",
    ),
    f(
        "string_to_array",
        "string, delimiter",
        "text[]",
        "Splits a string into an array.",
    ),
    f(
        "array_to_string",
        "array, delimiter",
        "text",
        "Joins an array into a string.",
    ),
    f(
        "array_length",
        "array, dimension",
        "integer",
        "The length of an array dimension.",
    ),
    f("unnest", "array", "setof", "Expands an array into rows."),
    f(
        "generate_series",
        "start, stop, [step]",
        "setof",
        "Rows of a sequence of numbers or timestamps.",
    ),
    f(
        "format",
        "template, value, ...",
        "text",
        "Builds text from a template with %s, %I and %L placeholders.",
    ),
    f("repeat", "string, count", "text", "Repeats a string."),
    f(
        "reverse",
        "string",
        "text",
        "Reverses the characters of a string.",
    ),
    f(
        "md5",
        "string",
        "text",
        "The MD5 hash of a string as hex text.",
    ),
    f(
        "to_char",
        "value, format",
        "text",
        "Formats a number or a timestamp as text.",
    ),
    f(
        "to_date",
        "text, format",
        "date",
        "Parses text into a date.",
    ),
    f(
        "to_timestamp",
        "text, format",
        "timestamptz",
        "Parses text into a timestamp.",
    ),
    f(
        "to_number",
        "text, format",
        "numeric",
        "Parses text into a number.",
    ),
    f(
        "date_trunc",
        "field, source",
        "timestamp",
        "Cuts a timestamp down to a precision such as day or month.",
    ),
    f(
        "date_part",
        "field, source",
        "double precision",
        "A field of a date or timestamp, such as the year.",
    ),
    f(
        "extract",
        "field FROM source",
        "numeric",
        "A field of a date or timestamp, such as the year.",
    ),
    f(
        "age",
        "timestamp, [timestamp]",
        "interval",
        "The interval between two timestamps, or from now.",
    ),
    f(
        "make_date",
        "year, month, day",
        "date",
        "Builds a date from its parts.",
    ),
    f(
        "now",
        "",
        "timestamptz",
        "The date and time the transaction started.",
    ),
    f(
        "clock_timestamp",
        "",
        "timestamptz",
        "The actual current time, changing during a statement.",
    ),
    f(
        "random",
        "",
        "double precision",
        "A random number from 0 up to 1.",
    ),
    f("gen_random_uuid", "", "uuid", "A new random UUID."),
    f(
        "ceil",
        "number",
        "number",
        "Rounds up to the next whole number.",
    ),
    f(
        "ceiling",
        "number",
        "number",
        "Rounds up to the next whole number.",
    ),
    f(
        "floor",
        "number",
        "number",
        "Rounds down to the previous whole number.",
    ),
    f(
        "trunc",
        "number, [digits]",
        "number",
        "Cuts a number off at the given decimal place.",
    ),
    f(
        "power",
        "base, exponent",
        "double precision",
        "A number raised to a power.",
    ),
    f(
        "sqrt",
        "number",
        "double precision",
        "The square root of a number.",
    ),
    f(
        "mod",
        "dividend, divisor",
        "number",
        "The remainder of a division.",
    ),
    f(
        "greatest",
        "value, ...",
        "",
        "The largest of the arguments.",
    ),
    f("least", "value, ...", "", "The smallest of the arguments."),
    f("to_jsonb", "value", "jsonb", "Converts a value to JSONB."),
    f(
        "jsonb_build_object",
        "key, value, ...",
        "jsonb",
        "Builds a JSONB object from key and value pairs.",
    ),
    f(
        "jsonb_extract_path_text",
        "json, path, ...",
        "text",
        "The text found by following a path in JSON.",
    ),
    f("row_to_json", "row", "json", "Converts a row to JSON."),
    f("pg_typeof", "value", "regtype", "The data type of a value."),
    sp(
        "current_user",
        "name",
        "The name of the current database user.",
    ),
    sp(
        "current_schema",
        "name",
        "The first schema of the search path.",
    ),
    sp(
        "localtimestamp",
        "timestamp",
        "The current date and time without a time zone.",
    ),
];

const ORACLE: &[Function] = &[
    agg(
        "listagg",
        "expression, [delimiter]",
        "varchar2",
        "Joins the values of the group into one string; used with WITHIN GROUP (ORDER BY ...).",
    ),
    agg(
        "median",
        "expression",
        "number",
        "The middle value of the group.",
    ),
    win(
        "row_number",
        "",
        "number",
        "The number of the row within its partition, starting at 1.",
    ),
    win(
        "rank",
        "",
        "number",
        "The rank of the row with gaps after ties.",
    ),
    win(
        "dense_rank",
        "",
        "number",
        "The rank of the row without gaps after ties.",
    ),
    win(
        "ntile",
        "buckets",
        "number",
        "Splits the partition into the given number of groups and returns the group of the row.",
    ),
    win(
        "lag",
        "value, [offset], [default]",
        "",
        "The value from an earlier row of the partition.",
    ),
    win(
        "lead",
        "value, [offset], [default]",
        "",
        "The value from a later row of the partition.",
    ),
    win(
        "first_value",
        "value",
        "",
        "The value from the first row of the window frame.",
    ),
    win(
        "last_value",
        "value",
        "",
        "The value from the last row of the window frame.",
    ),
    f(
        "nvl",
        "value, replacement",
        "",
        "Returns the replacement when the value is null.",
    ),
    f(
        "nvl2",
        "value, if_not_null, if_null",
        "",
        "Returns the second argument when the value is not null, else the third.",
    ),
    f(
        "decode",
        "expression, search, result, ..., [default]",
        "",
        "Compares an expression with search values and returns the matching result.",
    ),
    f(
        "to_char",
        "value, [format]",
        "varchar2",
        "Formats a number, date or timestamp as text.",
    ),
    f(
        "to_date",
        "text, [format]",
        "date",
        "Parses text into a date.",
    ),
    f(
        "to_number",
        "text, [format]",
        "number",
        "Parses text into a number.",
    ),
    f(
        "to_timestamp",
        "text, [format]",
        "timestamp",
        "Parses text into a timestamp.",
    ),
    f(
        "trunc",
        "value, [unit]",
        "",
        "Cuts a number or a date down to a precision.",
    ),
    f(
        "add_months",
        "date, months",
        "date",
        "Adds months to a date.",
    ),
    f(
        "months_between",
        "date1, date2",
        "number",
        "The number of months between two dates.",
    ),
    f(
        "last_day",
        "date",
        "date",
        "The last day of the month of a date.",
    ),
    f(
        "next_day",
        "date, weekday",
        "date",
        "The first given weekday after a date.",
    ),
    f(
        "extract",
        "field FROM source",
        "number",
        "A field of a date or timestamp, such as the year.",
    ),
    f(
        "substr",
        "string, start, [count]",
        "varchar2",
        "A part of a string from a position, with an optional length.",
    ),
    f(
        "instr",
        "string, search, [start], [occurrence]",
        "number",
        "The position of a substring in a string, 0 when absent.",
    ),
    f(
        "lpad",
        "string, length, [fill]",
        "varchar2",
        "Pads a string on the left to the given length.",
    ),
    f(
        "rpad",
        "string, length, [fill]",
        "varchar2",
        "Pads a string on the right to the given length.",
    ),
    f(
        "trim",
        "string",
        "varchar2",
        "Removes spaces (or the given character) from both ends of a string.",
    ),
    f(
        "initcap",
        "string",
        "varchar2",
        "Capitalizes the first letter of each word.",
    ),
    f(
        "translate",
        "string, from, to",
        "varchar2",
        "Replaces characters one by one.",
    ),
    f(
        "concat",
        "string1, string2",
        "varchar2",
        "Joins two strings.",
    ),
    f(
        "regexp_like",
        "string, pattern, [flags]",
        "boolean",
        "True when a string matches a regular expression.",
    ),
    f(
        "regexp_replace",
        "string, pattern, [replacement], [position], [occurrence], [flags]",
        "varchar2",
        "Replaces the part of a string that matches a regular expression.",
    ),
    f(
        "regexp_substr",
        "string, pattern, [position], [occurrence], [flags]",
        "varchar2",
        "The part of a string that matches a regular expression.",
    ),
    f(
        "regexp_instr",
        "string, pattern, [position], [occurrence], [return], [flags]",
        "number",
        "The position of a regular expression match.",
    ),
    f(
        "ceil",
        "number",
        "number",
        "Rounds up to the next whole number.",
    ),
    f(
        "floor",
        "number",
        "number",
        "Rounds down to the previous whole number.",
    ),
    f(
        "power",
        "base, exponent",
        "number",
        "A number raised to a power.",
    ),
    f("sqrt", "number", "number", "The square root of a number."),
    f(
        "mod",
        "dividend, divisor",
        "number",
        "The remainder of a division.",
    ),
    f(
        "greatest",
        "value, ...",
        "",
        "The largest of the arguments.",
    ),
    f("least", "value, ...", "", "The smallest of the arguments."),
    f("sys_guid", "", "raw", "A new globally unique identifier."),
    f(
        "sys_context",
        "namespace, parameter",
        "varchar2",
        "A value of the session context, such as the current schema.",
    ),
    sp(
        "sysdate",
        "date",
        "The current date and time of the database server.",
    ),
    sp(
        "systimestamp",
        "timestamp",
        "The current date and time of the database server with a time zone.",
    ),
    sp(
        "localtimestamp",
        "timestamp",
        "The current date and time in the session time zone.",
    ),
    sp("user", "varchar2", "The name of the current database user."),
    sp(
        "rownum",
        "number",
        "The number of the row in the order it was fetched, counted from 1.",
    ),
];

const SQLITE: &[Function] = &[
    agg(
        "group_concat",
        "expression, [delimiter]",
        "text",
        "Joins the values of the group into one string.",
    ),
    agg(
        "total",
        "expression",
        "real",
        "Adds up the values of the group; 0.0 for no rows.",
    ),
    win(
        "row_number",
        "",
        "integer",
        "The number of the row within its partition, starting at 1.",
    ),
    win(
        "rank",
        "",
        "integer",
        "The rank of the row with gaps after ties.",
    ),
    win(
        "dense_rank",
        "",
        "integer",
        "The rank of the row without gaps after ties.",
    ),
    win(
        "lag",
        "value, [offset], [default]",
        "",
        "The value from an earlier row of the partition.",
    ),
    win(
        "lead",
        "value, [offset], [default]",
        "",
        "The value from a later row of the partition.",
    ),
    f(
        "ifnull",
        "value, replacement",
        "",
        "Returns the replacement when the value is null.",
    ),
    f(
        "iif",
        "condition, if_true, if_false",
        "",
        "Returns the second argument when the condition is true, else the third.",
    ),
    f(
        "instr",
        "string, search",
        "integer",
        "The position of a substring in a string, 0 when absent.",
    ),
    f(
        "substr",
        "string, start, [count]",
        "text",
        "A part of a string from a position, with an optional length.",
    ),
    f(
        "trim",
        "string, [characters]",
        "text",
        "Removes characters (spaces by default) from both ends of a string.",
    ),
    f("hex", "blob", "text", "The hexadecimal text of a blob."),
    f(
        "quote",
        "value",
        "text",
        "The value written as an SQL literal.",
    ),
    f("typeof", "value", "text", "The storage class of a value."),
    f(
        "printf",
        "format, ...",
        "text",
        "Builds text from a template with % placeholders.",
    ),
    f(
        "format",
        "format, ...",
        "text",
        "Builds text from a template with % placeholders.",
    ),
    f("random", "", "integer", "A random 64-bit integer."),
    f("randomblob", "length", "blob", "A blob of random bytes."),
    f("zeroblob", "length", "blob", "A blob of zero bytes."),
    f(
        "unicode",
        "string",
        "integer",
        "The code point of the first character.",
    ),
    f(
        "char",
        "code, ...",
        "text",
        "A string built from code points.",
    ),
    f(
        "date",
        "time_value, [modifier], ...",
        "text",
        "The date of a time value as YYYY-MM-DD.",
    ),
    f(
        "time",
        "time_value, [modifier], ...",
        "text",
        "The time of a time value as HH:MM:SS.",
    ),
    f(
        "datetime",
        "time_value, [modifier], ...",
        "text",
        "The date and time of a time value.",
    ),
    f(
        "julianday",
        "time_value, [modifier], ...",
        "real",
        "The Julian day number of a time value.",
    ),
    f(
        "strftime",
        "format, time_value, [modifier], ...",
        "text",
        "Formats a time value with a template.",
    ),
    f(
        "json_extract",
        "json, path, ...",
        "",
        "The value found by a path in JSON.",
    ),
    f(
        "json_object",
        "label, value, ...",
        "text",
        "Builds a JSON object from labels and values.",
    ),
    f(
        "json_array",
        "value, ...",
        "text",
        "Builds a JSON array from values.",
    ),
    f(
        "changes",
        "",
        "integer",
        "The number of rows changed by the last statement.",
    ),
    f(
        "last_insert_rowid",
        "",
        "integer",
        "The rowid of the last inserted row.",
    ),
    f(
        "sqlite_version",
        "",
        "text",
        "The version of the SQLite library.",
    ),
    f(
        "max",
        "value, value, ...",
        "",
        "With two or more arguments: the largest of them.",
    ),
    f(
        "min",
        "value, value, ...",
        "",
        "With two or more arguments: the smallest of them.",
    ),
    f(
        "ceil",
        "number",
        "number",
        "Rounds up to the next whole number.",
    ),
    f(
        "floor",
        "number",
        "number",
        "Rounds down to the previous whole number.",
    ),
    sp("current_time", "text", "The current time (UTC)."),
];

/// A data type with the parameter list it may take.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DataType {
    pub name: &'static str,
    /// `(p, s)` when the type takes arguments.
    pub args: &'static str,
    pub doc: &'static str,
}

const fn t(name: &'static str, args: &'static str, doc: &'static str) -> DataType {
    DataType { name, args, doc }
}

const POSTGRES_TYPES: &[DataType] = &[
    t("smallint", "", "Two-byte integer."),
    t("integer", "", "Four-byte integer."),
    t("bigint", "", "Eight-byte integer."),
    t("serial", "", "Auto-incrementing four-byte integer."),
    t("bigserial", "", "Auto-incrementing eight-byte integer."),
    t(
        "numeric",
        "(precision, scale)",
        "Exact number with a chosen precision.",
    ),
    t("real", "", "Single-precision floating point number."),
    t(
        "double precision",
        "",
        "Double-precision floating point number.",
    ),
    t("boolean", "", "True or false."),
    t("text", "", "Text of any length."),
    t("varchar", "(length)", "Text up to a maximum length."),
    t("char", "(length)", "Fixed-length text, padded with spaces."),
    t("date", "", "Calendar date."),
    t("time", "", "Time of day."),
    t("timestamp", "", "Date and time without a time zone."),
    t("timestamptz", "", "Date and time with a time zone."),
    t("interval", "", "Span of time."),
    t("uuid", "", "Universally unique identifier."),
    t("json", "", "JSON stored as text."),
    t("jsonb", "", "JSON stored in a binary form, indexable."),
    t("bytea", "", "Binary data."),
    t("inet", "", "IPv4 or IPv6 host address."),
    t("cidr", "", "IPv4 or IPv6 network."),
    t("money", "", "Currency amount."),
    t("xml", "", "XML data."),
    t("tsvector", "", "Document prepared for full-text search."),
];

const ORACLE_TYPES: &[DataType] = &[
    t(
        "NUMBER",
        "(precision, scale)",
        "Number with a chosen precision and scale.",
    ),
    t("INTEGER", "", "Whole number, a NUMBER(38) subtype."),
    t("SMALLINT", "", "Whole number, a NUMBER(38) subtype."),
    t("FLOAT", "(precision)", "Floating point number."),
    t(
        "BINARY_FLOAT",
        "",
        "Single-precision floating point number.",
    ),
    t(
        "BINARY_DOUBLE",
        "",
        "Double-precision floating point number.",
    ),
    t("VARCHAR2", "(length)", "Variable-length text."),
    t(
        "NVARCHAR2",
        "(length)",
        "Variable-length text in the national character set.",
    ),
    t("CHAR", "(length)", "Fixed-length text, padded with spaces."),
    t(
        "NCHAR",
        "(length)",
        "Fixed-length text in the national character set.",
    ),
    t("CLOB", "", "Large text."),
    t("NCLOB", "", "Large text in the national character set."),
    t("BLOB", "", "Large binary data."),
    t("RAW", "(length)", "Binary data up to a maximum length."),
    t("DATE", "", "Date and time to the second."),
    t(
        "TIMESTAMP",
        "(precision)",
        "Date and time with fractional seconds.",
    ),
    t(
        "TIMESTAMP WITH TIME ZONE",
        "",
        "Timestamp with a time zone offset.",
    ),
    t("INTERVAL YEAR TO MONTH", "", "Span of years and months."),
    t(
        "INTERVAL DAY TO SECOND",
        "",
        "Span of days, hours, minutes and seconds.",
    ),
    t("ROWID", "", "Physical address of a row."),
    t("XMLTYPE", "", "XML data."),
    t("JSON", "", "JSON data."),
];

const SQLITE_TYPES: &[DataType] = &[
    t("INTEGER", "", "Whole number of up to eight bytes."),
    t("TEXT", "", "Text."),
    t("REAL", "", "Eight-byte floating point number."),
    t("BLOB", "", "Binary data stored as given."),
    t(
        "NUMERIC",
        "",
        "Number or text, converted to a number when it looks like one.",
    ),
    t("BOOLEAN", "", "Stored as 0 or 1."),
    t(
        "DATE",
        "",
        "Stored as text, real or integer by the application.",
    ),
    t(
        "DATETIME",
        "",
        "Stored as text, real or integer by the application.",
    ),
    t("VARCHAR", "(length)", "Text; the length is not enforced."),
    t(
        "DECIMAL",
        "(precision, scale)",
        "Number with numeric affinity.",
    ),
];

const GENERIC_TYPES: &[DataType] = &[
    t("integer", "", "Whole number."),
    t("bigint", "", "Large whole number."),
    t("numeric", "(precision, scale)", "Exact number."),
    t("varchar", "(length)", "Text up to a maximum length."),
    t("text", "", "Text of any length."),
    t("date", "", "Calendar date."),
    t("timestamp", "", "Date and time."),
    t("boolean", "", "True or false."),
];

fn dialect_functions(d: Dialect) -> &'static [Function] {
    match d {
        Dialect::Postgres => POSTGRES,
        Dialect::Oracle => ORACLE,
        Dialect::Sqlite => SQLITE,
        Dialect::Generic => &[],
    }
}

/// Every built-in function of a dialect: those of the dialect first, then
/// the common ones it does not redefine.
pub(crate) fn functions(d: Dialect) -> impl Iterator<Item = &'static Function> {
    let own = dialect_functions(d);
    let common = COMMON
        .iter()
        .filter(move |c| !own.iter().any(|o| o.name == c.name && o.params == c.params));
    own.iter().chain(common)
}

/// The overloads of a function by name (any letter case).
pub(crate) fn function(d: Dialect, name: &str) -> Vec<&'static Function> {
    functions(d)
        .filter(|f| f.name.eq_ignore_ascii_case(name))
        .collect()
}

pub(crate) fn types(d: Dialect) -> &'static [DataType] {
    match d {
        Dialect::Postgres => POSTGRES_TYPES,
        Dialect::Oracle => ORACLE_TYPES,
        Dialect::Sqlite => SQLITE_TYPES,
        Dialect::Generic => GENERIC_TYPES,
    }
}

pub(crate) fn data_type(d: Dialect, name: &str) -> Option<&'static DataType> {
    types(d).iter().find(|t| t.name.eq_ignore_ascii_case(name))
}

/// Keywords of a column definition after its type.
pub(crate) const COLUMN_CONSTRAINTS: &[&str] = &[
    "NOT NULL",
    "NULL",
    "DEFAULT",
    "PRIMARY KEY",
    "UNIQUE",
    "CHECK",
    "REFERENCES",
    "CONSTRAINT",
    "GENERATED",
    "COLLATE",
];

/// Keywords that can begin an expression and are not in the parser's
/// expected set as such.
pub(crate) const EXPRESSION_KEYWORDS: &[&str] = &[
    "CASE", "NULL", "TRUE", "FALSE", "EXISTS", "CAST", "NOT", "DISTINCT", "ALL",
];

const KEYWORD_DOCS: &[(&str, &str)] = &[
    (
        "SELECT",
        "Reads rows: lists the columns or expressions to return.",
    ),
    (
        "FROM",
        "Names the tables, views and subqueries the rows come from.",
    ),
    (
        "WHERE",
        "Keeps only the rows for which the condition is true.",
    ),
    (
        "GROUP",
        "GROUP BY puts rows with equal values into groups for aggregate functions.",
    ),
    (
        "HAVING",
        "Keeps only the groups for which the condition is true.",
    ),
    ("ORDER", "ORDER BY sorts the result."),
    ("LIMIT", "Limits the number of rows returned."),
    (
        "OFFSET",
        "Skips the given number of rows before returning any.",
    ),
    (
        "FETCH",
        "FETCH FIRST n ROWS ONLY limits the number of rows returned.",
    ),
    ("DISTINCT", "Removes duplicate rows from the result."),
    (
        "ALL",
        "Keeps duplicates (the default); after UNION it joins the results without removing them.",
    ),
    (
        "AS",
        "Gives an alias to a column, a table or a subquery, or names a query in a definition.",
    ),
    ("JOIN", "Combines the rows of two tables by a condition."),
    (
        "INNER",
        "INNER JOIN keeps the row pairs that satisfy the join condition.",
    ),
    (
        "LEFT",
        "LEFT JOIN also keeps the rows of the left table that have no match.",
    ),
    (
        "RIGHT",
        "RIGHT JOIN also keeps the rows of the right table that have no match.",
    ),
    ("FULL", "FULL JOIN keeps unmatched rows of both tables."),
    (
        "CROSS",
        "CROSS JOIN pairs every row of one table with every row of the other.",
    ),
    (
        "NATURAL",
        "NATURAL JOIN joins on all columns with the same names.",
    ),
    ("ON", "Introduces the condition of a join."),
    (
        "USING",
        "Joins on the listed columns that both tables have.",
    ),
    (
        "UNION",
        "Joins the results of two queries, removing duplicates unless ALL is given.",
    ),
    ("INTERSECT", "Returns the rows that both queries return."),
    (
        "EXCEPT",
        "Returns the rows of the first query that the second one does not return.",
    ),
    (
        "MINUS",
        "Returns the rows of the first query that the second one does not return.",
    ),
    (
        "WITH",
        "Starts a list of named subqueries (common table expressions).",
    ),
    ("RECURSIVE", "Lets a WITH query refer to itself."),
    ("INSERT", "Adds rows to a table."),
    ("INTO", "Names the table that receives the rows."),
    ("VALUES", "Lists rows of literal values."),
    ("UPDATE", "Changes values in existing rows."),
    ("SET", "Lists the columns to change and their new values."),
    ("DELETE", "Removes rows from a table."),
    (
        "MERGE",
        "Inserts, updates or deletes rows depending on whether they match a source.",
    ),
    (
        "RETURNING",
        "Returns values of the rows that were inserted, updated or deleted.",
    ),
    ("CREATE", "Creates a database object."),
    ("ALTER", "Changes a database object."),
    ("DROP", "Removes a database object."),
    ("TRUNCATE", "Removes all rows of a table quickly."),
    ("TABLE", "A table: rows of values in named columns."),
    ("VIEW", "A stored query used like a table."),
    ("INDEX", "A structure that speeds up lookups on columns."),
    ("SEQUENCE", "A generator of numbers."),
    ("COLUMN", "A named field of a table."),
    ("CONSTRAINT", "Names a rule a table's data must follow."),
    (
        "PRIMARY",
        "PRIMARY KEY identifies each row of a table; it is unique and not null.",
    ),
    (
        "FOREIGN",
        "FOREIGN KEY requires values to exist in a column of another table.",
    ),
    (
        "REFERENCES",
        "Names the table and columns a foreign key points to.",
    ),
    (
        "UNIQUE",
        "Requires the values of the columns to be different in every row.",
    ),
    ("CHECK", "Requires a condition to hold for every row."),
    ("DEFAULT", "The value used when none is given."),
    ("NOT", "Negates a condition."),
    ("NULL", "The missing value."),
    ("AND", "True when both conditions are true."),
    ("OR", "True when at least one condition is true."),
    (
        "IN",
        "True when a value is in a list or in the result of a subquery.",
    ),
    (
        "BETWEEN",
        "True when a value is within a range, ends included.",
    ),
    ("LIKE", "Matches text against a pattern with % and _."),
    ("ILIKE", "LIKE that ignores letter case."),
    ("IS", "Compares with NULL, TRUE or FALSE."),
    ("EXISTS", "True when a subquery returns at least one row."),
    (
        "CASE",
        "Chooses a value by conditions: CASE WHEN ... THEN ... ELSE ... END.",
    ),
    (
        "WHEN",
        "A condition of a CASE expression or a MERGE clause.",
    ),
    ("THEN", "The result of a matching WHEN."),
    ("ELSE", "The result when no WHEN matches."),
    ("END", "Closes a CASE expression or a block."),
    (
        "CAST",
        "Converts a value to another data type: CAST(value AS type).",
    ),
    ("ASC", "Sorts ascending."),
    ("DESC", "Sorts descending."),
    (
        "NULLS",
        "NULLS FIRST or NULLS LAST places null values in the sort order.",
    ),
    (
        "OVER",
        "Makes an aggregate or ranking function work over a window of rows.",
    ),
    (
        "PARTITION",
        "PARTITION BY splits the rows of a window into groups.",
    ),
    (
        "LATERAL",
        "Lets a subquery in FROM refer to the tables before it.",
    ),
    ("BEGIN", "Starts a transaction or a block."),
    ("COMMIT", "Makes the changes of the transaction permanent."),
    ("ROLLBACK", "Undoes the changes of the transaction."),
    ("EXPLAIN", "Shows the execution plan of a statement."),
    ("GRANT", "Gives privileges to a user or role."),
    ("REVOKE", "Takes privileges away."),
];

pub(crate) fn keyword_doc(word: &str) -> Option<&'static str> {
    KEYWORD_DOCS
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(word))
        .map(|(_, d)| *d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_know_optional_and_variable_tails() {
        let round = function(Dialect::Postgres, "round");
        assert_eq!(round.len(), 1);
        let params = round[0].parameters();
        assert_eq!(params.len(), 2);
        assert!(!params[0].optional && params[1].optional);
        assert_eq!(params[1].name, "digits");
        assert_eq!(round[0].signature(), "round(number, [digits]) -> number");
    }

    #[test]
    fn dialects_redefine_common_functions() {
        let all = functions(Dialect::Sqlite)
            .filter(|f| f.name == "ltrim")
            .count();
        assert_eq!(all, 1);
        assert!(function(Dialect::Oracle, "NVL").len() == 1);
        assert!(function(Dialect::Postgres, "nvl").is_empty());
    }
}
