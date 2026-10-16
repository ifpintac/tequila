//! Saving and restoring the render queue (`docs/04-UX-SPEC.md` §11).
//!
//! §11 asks for a queue "with persistence", and the reason is the obvious one:
//! a queue is a list of overnight jobs, and an application that lost it when
//! the window closed would be one nobody queued anything in.
//!
//! It is saved beside the crash journal rather than inside the project, and
//! that is a deliberate difference from the render *settings*, which are
//! document state. A queue spans projects — three jobs from one file and two
//! from another is an ordinary evening — and putting it in a document would
//! mean each project remembering its own share of a list that is really one
//! list.
//!
//! # Failure is not fatal
//!
//! A queue that cannot be read is reported and replaced with an empty one. It
//! is a convenience, not the user's work: losing it costs a few minutes of
//! re-queuing, and refusing to start the application over it would cost far
//! more.

use crate::json::{self, Value};
use crate::json_write;
use crate::{FormatError, Object};
use tequila_scene::render::{JobState, RenderJob, RenderQueue};
use std::path::Path;

const FILE: &str = "queue.json";

/// Where the queue lives, given the journal directory.
pub fn path(directory: &Path) -> std::path::PathBuf {
    directory.join(FILE)
}

pub fn encode(queue: &RenderQueue) -> String {
    let mut root = Object::new();
    root.set("next_id", queue.next_id() as f64).set(
        "jobs",
        Value::Array(
            queue
                .jobs()
                .iter()
                .map(|job| {
                    let mut object = Object::new();
                    object
                        .set("id", job.id as f64)
                        .set("name", job.name.clone())
                        .set("camera", job.camera as f64)
                        .set("state", state_key(job.state))
                        .set("seconds", job.seconds as f32)
                        .set("samples", job.samples as f64)
                        .set("settings", crate::render_settings_to_json(&job.settings));
                    if let Some(output) = &job.output {
                        object.set("output", output.clone());
                    }
                    if !job.message.is_empty() {
                        object.set("message", job.message.clone());
                    }
                    object.build()
                })
                .collect(),
        ),
    );
    json_write::to_string(&root.build())
}

pub fn decode(text: &str) -> Result<RenderQueue, FormatError> {
    let root = json::parse(text.as_bytes())
        .map_err(|e| FormatError::Invalid(format!("the render queue: {e}")))?;
    let mut jobs = Vec::new();
    for entry in root.get("jobs").map(|v| v.as_array()).unwrap_or(&[]) {
        let Some(id) = entry.get("id").and_then(|v| v.as_index()) else {
            continue;
        };
        let settings = entry
            .get("settings")
            .map(crate::read_render_settings_value)
            .unwrap_or_default();
        jobs.push(RenderJob {
            id: id as u64,
            name: entry
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Render")
                .to_string(),
            camera: entry.get("camera").and_then(|v| v.as_index()).unwrap_or(0),
            settings,
            // A job that was mid-render when the application stopped goes back
            // in the queue rather than staying "Rendering" forever. Nothing is
            // rendering after a restart, whatever the file says.
            state: match entry.get("state").and_then(|v| v.as_str()) {
                Some("done") => JobState::Done,
                Some("failed") => JobState::Failed,
                Some("held") => JobState::Held,
                _ => JobState::Queued,
            },
            output: entry
                .get("output")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            message: entry
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            seconds: entry.get("seconds").and_then(|v| v.as_f32()).unwrap_or(0.0) as f64,
            samples: entry
                .get("samples")
                .and_then(|v| v.as_index())
                .unwrap_or(0) as u32,
        });
    }
    let next = root
        .get("next_id")
        .and_then(|v| v.as_index())
        .unwrap_or(1) as u64;
    Ok(RenderQueue::from_jobs(jobs, next))
}

pub fn save(directory: &Path, queue: &RenderQueue) -> Result<(), FormatError> {
    std::fs::create_dir_all(directory)
        .map_err(|e| FormatError::Io(format!("{} could not be made: {e}", directory.display())))?;
    let file = path(directory);
    std::fs::write(&file, encode(queue))
        .map_err(|e| FormatError::Io(format!("{} could not be written: {e}", file.display())))
}

/// Read the queue, or start an empty one.
///
/// A missing file is not an error — it is the first run. A *broken* one is
/// reported, because a user who queued forty jobs deserves to know they are
/// gone rather than to find an empty list and wonder.
pub fn load(directory: &Path) -> (RenderQueue, Option<String>) {
    let file = path(directory);
    let Ok(text) = std::fs::read_to_string(&file) else {
        return (RenderQueue::new(), None);
    };
    match decode(&text) {
        Ok(queue) => (queue, None),
        Err(error) => (
            RenderQueue::new(),
            Some(format!("The render queue could not be read: {error}")),
        ),
    }
}

fn state_key(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        // A job that was running when the application stopped is written as
        // queued, so the file never describes a state that cannot be true of a
        // process which is not running.
        JobState::Rendering => "queued",
        JobState::Done => "done",
        JobState::Failed => "failed",
        JobState::Held => "held",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::render::{OutputFormat, RenderSettings};

    fn queue() -> RenderQueue {
        let mut queue = RenderQueue::new();
        queue.push(
            "Hero shot",
            0,
            RenderSettings {
                width: 3840,
                height: 2160,
                max_samples: 512,
                max_seconds: Some(900),
                format: OutputFormat::Exr,
                write_passes: true,
                output_directory: "renders/final".into(),
                ..Default::default()
            },
        );
        let second = queue.push("Detail", 1, RenderSettings::default());
        let job = queue.get_mut(second).unwrap();
        job.state = JobState::Done;
        job.output = Some("renders/detail.png".into());
        job.seconds = 41.5;
        job.samples = 256;
        queue
    }

    /// A queue that is saved and read back is the same queue. Identifiers
    /// included: a job that came back with a different one would lose the
    /// finished image it points at.
    #[test]
    fn a_queue_survives_being_saved() {
        let original = queue();
        let restored = decode(&encode(&original)).unwrap();
        assert_eq!(restored.len(), original.len());
        for (a, b) in restored.jobs().iter().zip(original.jobs()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.name, b.name);
            assert_eq!(a.camera, b.camera);
            assert_eq!(a.settings, b.settings);
            assert_eq!(a.output, b.output);
            assert_eq!(a.samples, b.samples);
        }
        assert_eq!(restored.next_id(), original.next_id());
    }

    /// Nothing is rendering after a restart, whatever the file says. A job that
    /// was mid-render goes back in the queue rather than sitting at
    /// "Rendering" forever with nothing rendering it.
    #[test]
    fn an_interrupted_job_goes_back_in_the_queue() {
        let mut original = queue();
        let id = original.jobs()[0].id;
        original.get_mut(id).unwrap().state = JobState::Rendering;
        let restored = decode(&encode(&original)).unwrap();
        assert_eq!(restored.get(id).unwrap().state, JobState::Queued);
        assert_eq!(restored.next_queued(), Some(id));
    }

    /// A held job stays held, and a finished one stays finished — the two
    /// states a user set on purpose.
    #[test]
    fn deliberate_states_survive() {
        let mut original = queue();
        let id = original.jobs()[0].id;
        original.get_mut(id).unwrap().state = JobState::Held;
        let restored = decode(&encode(&original)).unwrap();
        assert_eq!(restored.get(id).unwrap().state, JobState::Held);
        assert_eq!(restored.jobs()[1].state, JobState::Done);
    }

    /// A broken file costs a few minutes of re-queuing and must not cost the
    /// application's start — but the user is told, rather than finding an empty
    /// list and wondering.
    #[test]
    fn a_broken_queue_is_reported_and_not_fatal() {
        let directory = std::env::temp_dir().join("tequila-queue-test");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(path(&directory), "{ not json at all").unwrap();
        let (queue, message) = load(&directory);
        assert!(queue.is_empty());
        assert!(message.is_some(), "a broken queue said nothing");
        let _ = std::fs::remove_file(path(&directory));
    }

    /// A missing file is the first run, not a fault.
    #[test]
    fn a_missing_queue_is_not_an_error() {
        let directory = std::env::temp_dir().join("tequila-queue-missing");
        let _ = std::fs::remove_dir_all(&directory);
        let (queue, message) = load(&directory);
        assert!(queue.is_empty());
        assert!(message.is_none());
    }

    #[test]
    fn saving_and_loading_round_trips_through_a_file() {
        let directory = std::env::temp_dir().join("tequila-queue-roundtrip");
        let _ = std::fs::remove_dir_all(&directory);
        let original = queue();
        save(&directory, &original).unwrap();
        let (restored, message) = load(&directory);
        assert!(message.is_none());
        assert_eq!(restored.len(), 2);
        assert_eq!(restored.jobs()[0].settings.width, 3840);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
