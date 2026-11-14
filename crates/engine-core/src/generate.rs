//! AI generation (AGENTS.md §8, Produce → Generate): requests to ACE-Step 1.5 running in the
//! optional local sidecar (`services/ml-sidecar/`), and the sidecar's lifecycle. Hosts own the
//! process and the HTTP calls; the types and the state machine live here so desktop, web and
//! the UI share them.

use crate::edit::EditSource;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An instrument ACE-Step's `lego` task can add (its track names).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Instrument {
    Drums,
    Bass,
    Guitar,
    Keyboard,
    Strings,
    Synth,
    Percussion,
    Brass,
    Woodwinds,
    BackingVocals,
}

impl Instrument {
    pub const ALL: [Self; 10] = [
        Self::Drums,
        Self::Bass,
        Self::Guitar,
        Self::Keyboard,
        Self::Strings,
        Self::Synth,
        Self::Percussion,
        Self::Brass,
        Self::Woodwinds,
        Self::BackingVocals,
    ];

    /// ACE-Step's track name.
    pub fn track_name(self) -> &'static str {
        match self {
            Self::Drums => "drums",
            Self::Bass => "bass",
            Self::Guitar => "guitar",
            Self::Keyboard => "keyboard",
            Self::Strings => "strings",
            Self::Synth => "synth",
            Self::Percussion => "percussion",
            Self::Brass => "brass",
            Self::Woodwinds => "woodwinds",
            Self::BackingVocals => "backing_vocals",
        }
    }
}

/// What to generate, and from which audio.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum GenerationTask {
    /// A whole backing under a vocal (ACE-Step `complete`).
    Backing { from: EditSource },
    /// One instrument in context of a vocal (`lego`).
    Lego {
        from: EditSource,
        instrument: Instrument,
    },
    /// The song's mix regenerated between `start` and `end` song seconds (`repaint`).
    Repaint { start: f64, end: f64 },
}

impl GenerationTask {
    /// ACE-Step's task type.
    pub fn task_type(&self) -> &'static str {
        match self {
            Self::Backing { .. } => "complete",
            Self::Lego { .. } => "lego",
            Self::Repaint { .. } => "repaint",
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Backing { .. } => "AI backing".to_string(),
            Self::Lego { instrument, .. } => {
                format!("AI {}", instrument.track_name().replace('_', " "))
            }
            Self::Repaint { start, end } => format!("AI repaint {start:.0}–{end:.0} s"),
        }
    }
}

/// The ACE-Step checkpoint every task uses: `lego` and `complete` need a base (not turbo or
/// SFT) model, and the 2B base fits a 16 GB Mac where XL-base (≥ 20 GB) does not.
pub const GENERATION_MODEL: &str = "acestep-v15-base";

/// One generation, as asked for and as kept with the produced track (model, prompt and seed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GenerationRequest {
    pub task: GenerationTask,
    /// The style caption.
    pub prompt: String,
    pub seed: u32,
    pub model: String,
    /// The song's tempo and key the generation was conditioned on.
    pub bpm: f64,
    pub key: String,
}

/// Where the generation backend is on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum GeneratorState {
    /// Not installed (the opt-in install is several GB).
    NotInstalled,
    /// Installing: `step` says which (the Python environment, then the weights).
    Installing { step: String },
    /// Installed and idle: nothing runs and no memory is held.
    Ready,
    /// The sidecar is running a generation (it exits when the generation ends).
    Running,
    /// The last install or run failed; `installed` says whether a retry can run straight away.
    Failed { message: String, installed: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum GeneratorEvent {
    Install,
    Step { step: String },
    Installed,
    Start,
    Finished,
    Fail { message: String },
    Uninstalled,
}

/// The state after `event`, or `None` when the event doesn't apply in `state`.
pub fn next(state: &GeneratorState, event: GeneratorEvent) -> Option<GeneratorState> {
    use GeneratorEvent as E;
    use GeneratorState as S;
    let failed = |message, installed| S::Failed { message, installed };
    match (state, event) {
        (S::NotInstalled | S::Failed { .. }, E::Install) => Some(S::Installing {
            step: "Preparing".to_string(),
        }),
        (S::Installing { .. }, E::Step { step }) => Some(S::Installing { step }),
        (S::Installing { .. }, E::Installed) => Some(S::Ready),
        (S::Installing { .. }, E::Fail { message }) => Some(failed(message, false)),
        (
            S::Ready
            | S::Failed {
                installed: true, ..
            },
            E::Start,
        ) => Some(S::Running),
        (S::Running, E::Finished) => Some(S::Ready),
        (S::Running, E::Fail { message }) => Some(failed(message, true)),
        (S::Ready | S::Failed { .. }, E::Uninstalled) => Some(S::NotInstalled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn lifecycle_installs_runs_and_recovers() {
        let s = GeneratorState::NotInstalled;
        assert_eq!(next(&s, GeneratorEvent::Start), None, "not installed");
        let s = next(&s, GeneratorEvent::Install).unwrap();
        let s = next(
            &s,
            GeneratorEvent::Step {
                step: "Weights".into(),
            },
        )
        .unwrap();
        assert_eq!(
            s,
            GeneratorState::Installing {
                step: "Weights".into()
            }
        );
        assert_eq!(next(&s, GeneratorEvent::Start), None, "still installing");
        let s = next(&s, GeneratorEvent::Installed).unwrap();
        let s = next(&s, GeneratorEvent::Start).unwrap();
        assert_eq!(s, GeneratorState::Running);
        assert_eq!(next(&s, GeneratorEvent::Start), None, "one run at a time");
        assert_eq!(next(&s, GeneratorEvent::Uninstalled), None);
        let s = next(
            &s,
            GeneratorEvent::Fail {
                message: "oom".into(),
            },
        )
        .unwrap();
        // A failed run leaves the install usable.
        let s = next(&s, GeneratorEvent::Start).unwrap();
        let s = next(&s, GeneratorEvent::Finished).unwrap();
        assert_eq!(s, GeneratorState::Ready);
        assert_eq!(
            next(&s, GeneratorEvent::Uninstalled),
            Some(GeneratorState::NotInstalled)
        );
    }

    #[test]
    fn a_failed_install_must_be_retried_before_running() {
        let s = next(&GeneratorState::NotInstalled, GeneratorEvent::Install).unwrap();
        let s = next(
            &s,
            GeneratorEvent::Fail {
                message: "net".into(),
            },
        )
        .unwrap();
        assert_eq!(next(&s, GeneratorEvent::Start), None);
        assert!(matches!(
            next(&s, GeneratorEvent::Install),
            Some(GeneratorState::Installing { .. })
        ));
    }

    #[test]
    fn tasks_map_to_acestep() {
        let from = EditSource::VocalStem;
        assert_eq!(GenerationTask::Backing { from }.task_type(), "complete");
        let lego = GenerationTask::Lego {
            from,
            instrument: Instrument::BackingVocals,
        };
        assert_eq!(lego.task_type(), "lego");
        assert_eq!(lego.label(), "AI backing vocals");
        assert_eq!(
            GenerationTask::Repaint {
                start: 10.0,
                end: 20.0
            }
            .task_type(),
            "repaint"
        );
    }
}
