//! The disk cache: round trip, bad files ignored, atomic writes, content.
use std::fs;

use db_core::DataSourceId;
use db_core::schema::Level;
use db_model::{CACHE_FORMAT, CACHE_VERSION, CacheMiss, Model, ModelCache, NameMatch};

use crate::fixture::{Rng, random_snapshot, shop, synthetic};

fn cache() -> (tempfile::TempDir, ModelCache) {
    let dir = tempfile::tempdir().unwrap();
    let cache = ModelCache::new(dir.path().join("db"));
    (dir, cache)
}

#[test]
fn model_cache_model_round_trip() {
    let (_dir, cache) = cache();
    let ds = DataSourceId::new("shop");
    assert!(matches!(cache.load(&ds), Err(CacheMiss::Missing)));
    let model = Model::from_snapshot(ds.clone(), shop());
    cache.store(&model).unwrap();
    let loaded = cache.load(&ds).unwrap();
    assert_eq!(loaded, model);
    // Ids are the same after a restart: tree state keyed by id survives.
    let orders = model
        .find("public", "orders", NameMatch::Exact, &[])
        .unwrap();
    assert_eq!(loaded.node(orders.id()).unwrap().name(), "orders");
    // Storing again replaces the file and leaves no temporary files.
    cache.store(&loaded).unwrap();
    let files: Vec<_> = fs::read_dir(cache.dir(&ds))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(files, ["model.json"]);
}

#[test]
fn random_models_round_trip() {
    let (_dir, cache) = cache();
    for seed in 0..50 {
        let mut rng = Rng::new(seed);
        let level = if seed % 3 == 0 {
            Level::Names
        } else {
            Level::Structure
        };
        let ds = DataSourceId::new(format!("ds {seed}/x"));
        let model = Model::from_snapshot(ds.clone(), random_snapshot(&mut rng, level));
        cache.store(&model).unwrap();
        assert_eq!(cache.load(&ds).unwrap(), model, "seed {seed}");
    }
}

#[test]
fn the_file_holds_the_model_and_the_data_source_id_only() {
    let (_dir, cache) = cache();
    let ds = DataSourceId::new("reporting");
    cache
        .store(&Model::from_snapshot(ds.clone(), shop()))
        .unwrap();
    let path = cache.model_path(&ds);
    assert!(path.starts_with(cache.root()));
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        ["data_source", "format", "schemas", "token", "version"]
    );
    assert_eq!(json["format"], CACHE_FORMAT);
    assert_eq!(json["version"], CACHE_VERSION);
    assert_eq!(json["data_source"], "reporting");
    // Nothing about the connection anywhere in the file.
    fn walk(v: &serde_json::Value, keys: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    keys.push(k.clone());
                    walk(v, keys);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, keys)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    walk(&json, &mut all);
    for forbidden in [
        "host", "port", "user", "password", "secret", "service", "url",
    ] {
        assert!(
            !all.iter().any(|k| k == forbidden),
            "{forbidden} in the cache file"
        );
    }
}

#[test]
fn damaged_old_and_foreign_files_are_ignored() {
    let (_dir, cache) = cache();
    let ds = DataSourceId::new("shop");
    let path = cache.model_path(&ds);
    fs::create_dir_all(path.parent().unwrap()).unwrap();

    let cases: Vec<(Vec<u8>, &str)> = vec![
        (b"".to_vec(), "corrupt"),
        (b"{not json".to_vec(), "corrupt"),
        (vec![0xff, 0xfe, 0x00, 0x12], "corrupt"),
        (b"[1, 2, 3]".to_vec(), "corrupt"),
        (
            br#"{"format":"something-else","version":1,"data_source":"shop","schemas":[]}"#.to_vec(),
            "corrupt",
        ),
        (
            br#"{"format":"pyoxide-db-model","version":0,"data_source":"shop","model":{"tables":[]}}"#
                .to_vec(),
            "version",
        ),
        (
            br#"{"format":"pyoxide-db-model","version":99,"data_source":"shop","schemas":[]}"#
                .to_vec(),
            "version",
        ),
        (
            br#"{"format":"pyoxide-db-model","version":1,"data_source":"other","schemas":[]}"#
                .to_vec(),
            "other",
        ),
        (
            br#"{"format":"pyoxide-db-model","version":1,"data_source":"shop","schemas":[{"info":1}]}"#
                .to_vec(),
            "corrupt",
        ),
    ];
    for (bytes, expected) in cases {
        fs::write(&path, &bytes).unwrap();
        let miss = cache.load(&ds).unwrap_err();
        let got = match miss {
            CacheMiss::Corrupt(_) => "corrupt",
            CacheMiss::Version(_) => "version",
            CacheMiss::OtherDataSource => "other",
            ref other => panic!("{other:?}"),
        };
        assert_eq!(got, expected, "{}", String::from_utf8_lossy(&bytes));
        assert!(!miss.to_string().is_empty());
    }
    // A cut-off file (a crash while writing elsewhere) is damaged, not fatal.
    let model = Model::from_snapshot(ds.clone(), shop());
    cache.store(&model).unwrap();
    let full = fs::read(&path).unwrap();
    for cut in [1, full.len() / 3, full.len() - 1] {
        fs::write(&path, &full[..cut]).unwrap();
        assert!(matches!(cache.load(&ds), Err(CacheMiss::Corrupt(_))));
    }
    // The next store repairs it.
    cache.store(&model).unwrap();
    assert_eq!(cache.load(&ds).unwrap(), model);
}

#[test]
fn a_cancelled_load_gives_nothing() {
    let (_dir, cache) = cache();
    let ds = DataSourceId::new("big");
    cache
        .store(&Model::from_snapshot(ds.clone(), synthetic(2_000)))
        .unwrap();
    assert!(matches!(
        cache.load_cancellable(&ds, &|| true),
        Err(CacheMiss::Cancelled)
    ));
    assert_eq!(cache.load(&ds).unwrap().schemas().len(), 2);
}

#[test]
fn data_sources_have_folders_of_their_own_and_can_be_forgotten() {
    let (_dir, cache) = cache();
    let a = DataSourceId::new("a/../b");
    let b = DataSourceId::new("b");
    cache
        .store(&Model::from_snapshot(a.clone(), shop()))
        .unwrap();
    cache.store(&Model::empty(b.clone())).unwrap();
    assert_ne!(cache.dir(&a), cache.dir(&b));
    assert!(cache.dir(&a).starts_with(cache.root()));
    assert_eq!(cache.dir(&a).parent(), Some(cache.root()));
    assert!(cache.load(&b).unwrap().is_empty());
    cache.remove(&a).unwrap();
    assert!(matches!(cache.load(&a), Err(CacheMiss::Missing)));
    cache.remove(&a).unwrap();
    assert!(cache.load(&b).is_ok());
}

#[test]
fn the_user_cache_is_outside_the_project() {
    // `PYOXIDE_CACHE_DIR` is not set by the tests; the folder is the user's
    // cache folder or nothing.
    if let Some(cache) = ModelCache::user() {
        assert!(cache.root().ends_with("db"));
        assert!(cache.root().is_absolute() || std::env::var_os("PYOXIDE_CACHE_DIR").is_some());
    }
}
