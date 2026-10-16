//! What one render on the farm is: which shot of the project, at what size,
//! and what kind of picture comes back. The project itself travels beside
//! it, as a bundle (`crate::bundle`).

use tequila_format::json::Value;
use tequila_format::json_write::Object;

/// A render, as the command-line renderer's options describe it - which is
/// what a worker runs it with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobSpec {
    /// Shown in the monitor, and in the worker's log.
    pub name: String,
    /// Unset means the project's own.
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub samples: Option<u32>,
    /// A saved camera, Studio, or configurator choice, by name or number,
    /// as `tequila --camera`, `--studio` and `--configuration` take them.
    pub camera: Option<String>,
    pub studio: Option<String>,
    pub model_set: Option<String>,
    pub configuration: Option<String>,
    /// `png` or `jpg`.
    pub format: String,
    /// `cycles` to render with Cycles where the worker has it; else
    /// Tequila's own.
    pub engine: Option<String>,
    pub alpha: bool,
    pub silhouette: bool,
}

impl JobSpec {
    pub fn to_json(&self) -> Value {
        let mut out = Object::new();
        out.set("name", self.name.clone()).set("format", self.format.clone()).set("alpha", self.alpha).set("silhouette", self.silhouette);
        out.set_some("width", self.width.map(|v| v as f64))
            .set_some("height", self.height.map(|v| v as f64))
            .set_some("samples", self.samples.map(|v| v as f64))
            .set_some("camera", self.camera.clone())
            .set_some("studio", self.studio.clone())
            .set_some("model_set", self.model_set.clone())
            .set_some("configuration", self.configuration.clone())
            .set_some("engine", self.engine.clone());
        out.build()
    }

    pub fn from_json(value: &Value) -> Result<JobSpec, String> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        let count = |key: &str| value.get(key).and_then(Value::as_f64).filter(|v| *v >= 1.0 && *v <= 1.0e6).map(|v| v as usize);
        let format = text("format").unwrap_or_else(|| "png".into());
        if !matches!(format.as_str(), "png" | "jpg") {
            return Err(format!("`{format}` is not a picture format a worker writes: png or jpg"));
        }
        Ok(JobSpec {
            name: text("name").unwrap_or_default(),
            width: count("width"),
            height: count("height"),
            samples: count("samples").map(|v| v as u32),
            camera: text("camera"),
            studio: text("studio"),
            model_set: text("model_set"),
            configuration: text("configuration"),
            format,
            engine: text("engine"),
            alpha: value.get("alpha").and_then(Value::as_bool).unwrap_or(false),
            silhouette: value.get("silhouette").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// A header as one line of JSON.
pub fn header(value: &Value) -> String {
    tequila_format::json_write::to_string(value).lines().map(str::trim).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_round_trips() {
        let job = JobSpec {
            name: "Red, Leather".into(),
            width: Some(1920),
            height: Some(1080),
            samples: Some(256),
            configuration: Some("Red,Leather".into()),
            format: "jpg".into(),
            engine: Some("cycles".into()),
            silhouette: true,
            ..JobSpec::default()
        };
        let text = header(&job.to_json());
        assert!(!text.contains('\n'));
        let back = JobSpec::from_json(&tequila_format::json::parse(text.as_bytes()).unwrap()).unwrap();
        assert_eq!(back, job);
    }

    #[test]
    fn a_format_a_worker_does_not_write_is_refused() {
        let value = tequila_format::json::parse(br#"{"format":"exe"}"#).unwrap();
        assert!(JobSpec::from_json(&value).is_err());
    }
}
