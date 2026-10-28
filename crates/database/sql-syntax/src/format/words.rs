//! Names of built-in types and functions, for the case options. Sorted,
//! upper case; written from the public manuals of the engines.

const TYPES: &[&str] = &[
    "ARRAY",
    "BIGINT",
    "BIGSERIAL",
    "BINARY",
    "BIT",
    "BLOB",
    "BOOL",
    "BOOLEAN",
    "BYTEA",
    "CHAR",
    "CHARACTER",
    "CLOB",
    "DATE",
    "DATETIME",
    "DEC",
    "DECIMAL",
    "DOUBLE",
    "FLOAT",
    "INT",
    "INT2",
    "INT4",
    "INT8",
    "INTEGER",
    "INTERVAL",
    "JSON",
    "JSONB",
    "LONG",
    "MONEY",
    "NCHAR",
    "NCLOB",
    "NUMBER",
    "NUMERIC",
    "NVARCHAR",
    "NVARCHAR2",
    "PRECISION",
    "RAW",
    "REAL",
    "ROWID",
    "SERIAL",
    "SMALLINT",
    "SMALLSERIAL",
    "TEXT",
    "TIME",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TINYINT",
    "UUID",
    "VARBINARY",
    "VARCHAR",
    "VARCHAR2",
    "VARYING",
    "XML",
    "XMLTYPE",
    "ZONE",
];

const FUNCTIONS: &[&str] = &[
    "ABS",
    "AVG",
    "CEIL",
    "CEILING",
    "COALESCE",
    "CONCAT",
    "COUNT",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "DATE_TRUNC",
    "DENSE_RANK",
    "EXTRACT",
    "FIRST_VALUE",
    "FLOOR",
    "GREATEST",
    "IFNULL",
    "INSTR",
    "LAG",
    "LAST_VALUE",
    "LEAD",
    "LEAST",
    "LENGTH",
    "LOWER",
    "LPAD",
    "LTRIM",
    "MAX",
    "MIN",
    "MOD",
    "NOW",
    "NULLIF",
    "NVL",
    "NVL2",
    "POSITION",
    "POWER",
    "RANK",
    "REPLACE",
    "ROUND",
    "ROW_NUMBER",
    "RPAD",
    "RTRIM",
    "SQRT",
    "STRING_AGG",
    "SUBSTR",
    "SUBSTRING",
    "SUM",
    "SYSDATE",
    "TO_CHAR",
    "TO_DATE",
    "TO_NUMBER",
    "TRIM",
    "TRUNC",
    "UPPER",
];

fn contains(table: &[&str], word: &str) -> bool {
    let mut buf = [0u8; 24];
    if word.len() > buf.len() || !word.is_ascii() {
        return false;
    }
    let upper = &mut buf[..word.len()];
    upper.copy_from_slice(word.as_bytes());
    upper.make_ascii_uppercase();
    std::str::from_utf8(upper).is_ok_and(|w| table.binary_search(&w).is_ok())
}

pub(crate) fn is_builtin_type(word: &str) -> bool {
    contains(TYPES, word)
}

pub(crate) fn is_builtin_function(word: &str) -> bool {
    contains(FUNCTIONS, word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_upper_case() {
        for t in [TYPES, FUNCTIONS] {
            assert!(t.windows(2).all(|w| w[0] < w[1]));
            assert!(t.iter().all(|w| w.bytes().all(|b| !b.is_ascii_lowercase())));
        }
    }

    #[test]
    fn lookup_ignores_case() {
        assert!(is_builtin_type("VarChar"));
        assert!(is_builtin_function("count"));
        assert!(!is_builtin_function("my_func"));
    }
}
