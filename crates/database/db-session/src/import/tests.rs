use std::path::Path;

use super::*;

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn the_format_of_a_file_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let semi = write(dir.path(), "a.csv", b"id;name\n1;Ada\n2;Grace\n");
    let (format, encoding) = detect_format(&semi).unwrap();
    assert_eq!(format.csv.delimiter, b';');
    assert_eq!(encoding, Encoding::Utf8);
    let tsv = write(dir.path(), "a.tsv", b"id\n1\n");
    assert_eq!(detect_format(&tsv).unwrap().0.csv.delimiter, b'\t');
    let latin = write(dir.path(), "b.csv", b"id,name\n1,Caf\xe9\n");
    assert_eq!(detect_format(&latin).unwrap().1, Encoding::Windows1252);
}

#[test]
fn the_preview_follows_the_options() {
    let dir = tempfile::tempdir().unwrap();
    let text = "id,name,note\n1,\"Ada, L.\",\n2,\"Line\nbreak\",\"say \"\"hi\"\" now\"\n3,Grace,\"\"\n4,x,y\n";
    let path = write(dir.path(), "a.csv", text.as_bytes());
    let p = preview(&path, &ImportFormat::default(), None, 3).unwrap();
    assert_eq!(p.header, ["id", "name", "note"]);
    assert!(p.more);
    assert_eq!(
        p.rows,
        vec![
            vec![Some("1".into()), Some("Ada, L.".into()), None],
            vec![
                Some("2".into()),
                Some("Line\nbreak".into()),
                Some("say \"hi\" now".into())
            ],
            vec![Some("3".into()), Some("Grace".into()), Some(String::new())],
        ]
    );
    // Without a header the first record is data and the columns are named.
    let format = ImportFormat {
        header: false,
        ..ImportFormat::default()
    };
    let p = preview(&path, &format, None, 1).unwrap();
    assert_eq!(p.header, ["Column1", "Column2", "Column3"]);
    assert_eq!(p.rows[0][0].as_deref(), Some("id"));
}

#[test]
fn null_text_and_empty_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "n.csv", b"a,b\n\\N,\n\"\\N\",\"\"\n");
    let format = ImportFormat {
        null_text: Some("\\N".into()),
        ..ImportFormat::default()
    };
    let p = preview(&path, &format, None, 10).unwrap();
    assert_eq!(p.rows[0], [None, None]);
    // Quoted: texts, not NULL.
    assert_eq!(p.rows[1], [Some("\\N".into()), Some(String::new())]);
    let format = ImportFormat {
        empty_as_null: false,
        ..ImportFormat::default()
    };
    let p = preview(&path, &format, None, 10).unwrap();
    assert_eq!(p.rows[0], [Some("\\N".into()), Some(String::new())]);
}

#[test]
fn columns_are_paired_by_name_or_position() {
    let table: Vec<String> = ["id", "Name", "amount", "extra"].map(String::from).to_vec();
    let file: Vec<String> = ["amount", "name", "id", "skipped"]
        .map(String::from)
        .to_vec();
    let map = auto_map(&table, &file, true);
    assert_eq!(
        map.iter().map(|m| m.source).collect::<Vec<_>>(),
        [Some(2), Some(1), Some(0), None]
    );
    let map = auto_map(&table, &["Column1".into(), "Column2".into()], false);
    assert_eq!(
        map.iter().map(|m| m.source).collect::<Vec<_>>(),
        [Some(0), Some(1), None, None]
    );
}
