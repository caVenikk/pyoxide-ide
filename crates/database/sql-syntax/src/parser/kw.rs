// The keywords the parser looks at, in byte order of their text (the lookup
// is a binary search; a test checks the order). A word that is not here is
// a name to the parser, whatever the dialect's keyword tables say.
macro_rules! keywords {
    ($($kw:ident)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub(crate) enum Kw {
            $($kw,)*
        }

        const TEXTS: &[&str] = &[$(stringify!($kw),)*];
        const ALL: &[Kw] = &[$(Kw::$kw,)*];
    };
}

keywords! {
    ALL AND ANY APPLY ARRAY AS ASC ASYMMETRIC AT BETWEEN BOTH BY CASE CAST COLLATE CONNECT
    CONNECT_BY_ROOT CROSS CURRENT CYCLE DESC DISTINCT ELSE END ESCAPE EXCEPT EXCLUDE EXISTS
    EXTRACT FALSE FETCH FILTER FIRST FOLLOWING FOR FROM FULL GLOB GROUP GROUPING GROUPS HAVING
    IGNORE ILIKE IN INCLUDE INDEXED INNER INTERSECT INTERVAL INTO IS ISNULL JOIN KEEP KEY LAST
    LATERAL LEADING LEFT LIKE LIMIT LOCAL MATCH MATCH_RECOGNIZE MATERIALIZED MINUS MODEL NATURAL
    NEXT NO NOCYCLE NOT NOTNULL NULL NULLS OF OFFSET ON ONLY OR ORDER ORDINALITY OTHERS OUTER
    OVER OVERLAY PARTITION PERCENT PIVOT PLACING POSITION PRECEDING PRIOR RANGE RECURSIVE REGEXP
    RESPECT RIGHT ROW ROWS SAMPLE SEARCH SELECT SETS SHARE SIBLINGS SIMILAR SKIP SOME START
    SUBSTRING SYMMETRIC TABLE TABLESAMPLE THEN TIES TIME TO TRAILING TRIM TRUE UNBOUNDED UNION
    UNIQUE UNKNOWN UNPIVOT UPDATE USING VALUES WHEN WHERE WINDOW WITH WITHIN WITHOUT XML ZONE
}

const LONGEST: usize = 24;

impl Kw {
    pub(crate) fn text(self) -> &'static str {
        TEXTS[self as usize]
    }

    /// The keyword `word` is, ignoring case.
    pub(crate) fn lookup(word: &str) -> Option<Kw> {
        if word.len() > LONGEST || !word.is_ascii() {
            return None;
        }
        let mut buf = [0u8; LONGEST];
        let upper = &mut buf[..word.len()];
        upper.copy_from_slice(word.as_bytes());
        upper.make_ascii_uppercase();
        let upper = std::str::from_utf8(upper).ok()?;
        TEXTS.binary_search(&upper).ok().map(|i| ALL[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_fits() {
        assert!(TEXTS.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
        assert!(TEXTS.iter().all(|t| t.len() <= LONGEST));
        for (i, kw) in ALL.iter().enumerate() {
            assert_eq!(*kw as usize, i);
            assert_eq!(Kw::lookup(&kw.text().to_lowercase()), Some(*kw));
        }
        assert_eq!(Kw::lookup("customers"), None);
        assert_eq!(Kw::lookup("sélect"), None);
    }
}
