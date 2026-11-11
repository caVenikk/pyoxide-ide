//! `Option<f32>` fields written as the shortest decimal that reads back
//! as the same `f32` (`12.6`, not the `f64` of the `f32`:
//! `12.600000381469727`), for settings people read in `settings.toml`.

use serde::Serializer;

pub(crate) fn serialize<S: Serializer>(
    value: &Option<f32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(v) => {
            serializer.serialize_some(&v.to_string().parse::<f64>().unwrap_or(f64::from(*v)))
        }
        None => serializer.serialize_none(),
    }
}
