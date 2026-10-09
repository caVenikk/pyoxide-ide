//! SQL used to read schema metadata. Written from the public Oracle data
//! dictionary documentation (ALL_* views).

/// Tables visible to the current user.
pub const ORACLE_LIST_TABLES: &str = "\
SELECT owner, table_name FROM all_tables \
WHERE owner NOT IN ('SYS','SYSTEM','XDB','CTXSYS','MDSYS','ORDSYS','OUTLN','DBSNMP','APPQOSSYS','AUDSYS','GSMADMIN_INTERNAL','LBACSYS','DVSYS','OJVMSYS','WMSYS') \
ORDER BY owner, table_name";

/// Columns of one table. Identifiers are strictly validated (letters, digits,
/// `_`, `$`, `#`) before being inlined; returns `None` for anything else.
pub fn oracle_describe_table(owner: &str, table: &str) -> Option<String> {
    fn ok(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 128
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '#'))
    }
    if !ok(owner) || !ok(table) {
        return None;
    }
    Some(format!(
        "SELECT column_name, data_type, data_length, nullable \
         FROM all_tab_columns WHERE owner = '{}' AND table_name = '{}' \
         ORDER BY column_id",
        owner.to_ascii_uppercase(),
        table.to_ascii_uppercase()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_rejects_injection() {
        assert!(oracle_describe_table("HR", "EMP' OR '1'='1").is_none());
        assert!(oracle_describe_table("", "EMP").is_none());
    }

    #[test]
    fn describe_accepts_plain_names() {
        let sql = oracle_describe_table("hr", "employees").unwrap();
        assert!(sql.contains("owner = 'HR'"));
        assert!(sql.contains("table_name = 'EMPLOYEES'"));
    }
}
