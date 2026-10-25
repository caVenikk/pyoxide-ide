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
    ADD AFTER ALL ALTER AND ANY APPLY ARRAY AS ASC ASYMMETRIC AT AUTHID BEFORE BEGIN BETWEEN
    BOTH BULK BY CASCADE CASE CAST CHECK CLOSE COLLATE COLLECT COLUMN COMMENT CONFLICT CONNECT
    CONNECT_BY_ROOT CONSTANT CONSTRAINT CONTINUE CREATE CROSS CURRENT CURSOR CYCLE DECLARE
    DEFAULT DELETE DESC DISTINCT DO DROP EACH ELSE ELSEIF ELSIF END ESCAPE EXCEPT EXCEPTION
    EXCLUDE EXECUTE EXISTS EXIT EXTRACT FALSE FETCH FILTER FIRST FOLLOWING FOR FORALL FOREACH
    FOREIGN FROM FULL FUNCTION GENERATED GLOB GOTO GROUP GROUPING GROUPS HAVING IF IGNORE ILIKE
    IMMEDIATE IN INCLUDE INDEX INDEXED INNER INSERT INSTEAD INTERSECT INTERVAL INTO IS ISNULL
    JOIN KEEP KEY LAST LATERAL LEADING LEFT LIKE LIMIT LOCAL LOOP MATCH MATCHED MATCH_RECOGNIZE
    MATERIALIZED MERGE MINUS MODEL NATURAL NEXT NO NOCYCLE NOT NOTHING NOTNULL NULL NULLS OF
    OFFSET ON ONLY OPEN OR ORDER ORDINALITY OTHERS OUTER OVER OVERLAY PACKAGE PARTITION PERCENT
    PERFORM PIVOT PLACING POSITION PRAGMA PRECEDING PRIMARY PRIOR PROCEDURE RAISE RANGE
    RECURSIVE REFERENCES REGEXP REPLACE RESPECT RETURN RETURNING RETURNS REVERSE RIGHT ROW ROWS
    SAMPLE SEARCH SELECT SEQUENCE SET SETS SHARE SIBLINGS SIMILAR SKIP SOME START SUBSTRING
    SYMMETRIC TABLE TABLESAMPLE TEMP TEMPORARY THEN TIES TIME TO TRAILING TRIGGER TRIM TRUE
    TRUNCATE TYPE UNBOUNDED UNION UNIQUE UNKNOWN UNPIVOT UPDATE USING VALUES VIEW WHEN WHERE
    WHILE WINDOW WITH WITHIN WITHOUT XML ZONE
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
