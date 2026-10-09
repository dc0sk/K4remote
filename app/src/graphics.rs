//! Which renderer the app starts with (FR-UI-27): the GRAPHICS setting, the environment
//! overrides, and whether a GPU adapter exists, decided once before iced starts. Pure apart from
//! [`apply_at_start`]; the design is `docs/concept/large-screen-3d-plan.md` v0.2 §4.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use k4_config::Renderer;

/// iced's own switch: it reads this when it starts and tries the backends listed, in order.
pub const ICED_BACKEND: &str = "ICED_BACKEND";
/// The app's diagnosis override for the waterfall path (`cpu` / `gpu`).
pub const K4_WATERFALL: &str = "K4_WATERFALL";

/// Why the renderer is what it is — for the GRAPHICS tab's status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Auto: a GPU adapter was found (or not).
    Detected,
    /// Chosen in Settings.
    Chosen,
    /// GPU chosen, but no adapter: started on the CPU instead.
    NoAdapter,
    /// `ICED_BACKEND` set in the environment.
    EnvIcedBackend,
    /// `K4_WATERFALL` set in the environment.
    EnvK4Waterfall,
}

/// The decision made at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// What to set `ICED_BACKEND` to before iced starts; `None` = leave it as it is.
    pub set_iced_backend: Option<&'static str>,
    /// Whether the GPU waterfall (and the GPU 3D view) may be used.
    pub gpu: bool,
    pub reason: Reason,
}

/// Decide. `iced_env` / `k4_env` are the environment's values before the app touches them;
/// `adapter` reports whether a wgpu adapter exists (only asked when it matters).
///
/// - The user's own `ICED_BACKEND` wins; the GPU paths are used only if it can be wgpu.
/// - GPU sets `wgpu,tiny-skia`, so iced falls back to software instead of exiting when there is
///   no adapter; CPU sets `tiny-skia`; Auto leaves iced to choose.
/// - `K4_WATERFALL=cpu|gpu` still forces the waterfall path — but never the GPU path under the
///   software renderer, where a shader widget draws nothing.
pub fn decide(
    setting: Renderer,
    iced_env: Option<&str>,
    k4_env: Option<&str>,
    adapter: impl Fn() -> bool,
) -> Decision {
    let user_backend = iced_env.map(str::trim).filter(|v| !v.is_empty());
    let (set, software_only, reason) = match (user_backend, setting) {
        (Some(v), _) => (
            None,
            !v.split(',').any(|b| b.trim() == "wgpu"),
            Reason::EnvIcedBackend,
        ),
        (None, Renderer::Cpu) => (Some("tiny-skia"), true, Reason::Chosen),
        (None, Renderer::Gpu) => (Some("wgpu,tiny-skia"), false, Reason::Chosen),
        (None, Renderer::Auto) => (None, false, Reason::Detected),
    };
    if software_only {
        return Decision {
            set_iced_backend: set,
            gpu: false,
            reason,
        };
    }
    let forced = match k4_env.map(str::trim) {
        Some("cpu") => Some(false),
        Some("gpu") => Some(true),
        _ => None,
    };
    if let Some(gpu) = forced {
        // Forcing the GPU waterfall still needs an adapter, or iced runs on the software renderer.
        let gpu = gpu && adapter();
        return Decision {
            set_iced_backend: set,
            gpu,
            reason: Reason::EnvK4Waterfall,
        };
    }
    let found = adapter();
    let reason = match (setting, reason, found) {
        (Renderer::Gpu, Reason::Chosen, false) => Reason::NoAdapter,
        _ => reason,
    };
    Decision {
        set_iced_backend: set,
        gpu: found,
        reason,
    }
}

static DECISION: OnceLock<Decision> = OnceLock::new();

/// Decide from the saved setting and the environment, and set `ICED_BACKEND` — **before iced
/// starts**, while `main` is still the only thread. Returns the decision; later calls return the
/// same one.
pub fn apply_at_start(setting: Renderer, adapter: impl Fn() -> bool) -> Decision {
    *DECISION.get_or_init(|| {
        let iced = std::env::var(ICED_BACKEND).ok();
        let k4 = std::env::var(K4_WATERFALL).ok();
        // What to set does not depend on the adapter, so set it first: the probe creates a wgpu
        // instance, and a driver may start threads, after which `set_var` would race their reads.
        let set = decide(setting, iced.as_deref(), k4.as_deref(), || false).set_iced_backend;
        if let Some(v) = set {
            // Edition 2021, and no other thread exists yet (iced has not started, nothing probed).
            std::env::set_var(ICED_BACKEND, v);
        }
        decide(setting, iced.as_deref(), k4.as_deref(), adapter)
    })
}

/// The decision made at start, if [`apply_at_start`] ran (it does, in `main`).
pub fn decision() -> Option<Decision> {
    DECISION.get().copied()
}

/// Set by a GPU primitive's `prepare`, which only runs under wgpu: the honest "the GPU is drawing".
pub static GPU_DRAWING: AtomicBool = AtomicBool::new(false);

/// Note that a GPU primitive was prepared.
pub fn note_gpu_drawing() {
    GPU_DRAWING.store(true, Ordering::Relaxed);
}

/// The GRAPHICS tab's status line: what is drawing now, and why.
pub fn status_text(d: &Decision, gpu_drawing: bool) -> String {
    let why = match d.reason {
        Reason::Detected if d.gpu => "a GPU was found",
        Reason::Detected => "no GPU was found",
        Reason::Chosen => "chosen in Settings",
        Reason::NoAdapter => "GPU chosen, but none was found",
        Reason::EnvIcedBackend => "set by ICED_BACKEND in the environment",
        Reason::EnvK4Waterfall => "set by K4_WATERFALL in the environment",
    };
    match (d.gpu, gpu_drawing) {
        (true, true) => format!("In use: GPU (wgpu) — {why}."),
        (true, false) => format!("GPU selected — {why}; waiting for the first frame."),
        (false, _) => format!("In use: CPU (software) — {why}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-UI-27: the whole decision table — setting × the user's ICED_BACKEND × K4_WATERFALL ×
    /// adapter present.
    /// trace: FR-UI-27
    #[test]
    fn fr_ui_27_renderer_decision_table() {
        use Reason::*;
        use Renderer::*;
        let d = |s, i, k, a: bool| decide(s, i, k, move || a);
        // Auto: iced chooses; the GPU paths follow the adapter.
        assert_eq!(
            d(Auto, None, None, true),
            Decision {
                set_iced_backend: None,
                gpu: true,
                reason: Detected
            }
        );
        assert_eq!(
            d(Auto, None, None, false),
            Decision {
                set_iced_backend: None,
                gpu: false,
                reason: Detected
            }
        );
        // GPU: wgpu with a software fallback, never wgpu alone (iced would exit without an adapter).
        assert_eq!(
            d(Gpu, None, None, true),
            Decision {
                set_iced_backend: Some("wgpu,tiny-skia"),
                gpu: true,
                reason: Chosen
            }
        );
        assert_eq!(
            d(Gpu, None, None, false),
            Decision {
                set_iced_backend: Some("wgpu,tiny-skia"),
                gpu: false,
                reason: NoAdapter
            }
        );
        // CPU: software, whatever the adapter, and the adapter is not even asked.
        assert_eq!(
            decide(Cpu, None, None, || panic!("no probe needed")),
            Decision {
                set_iced_backend: Some("tiny-skia"),
                gpu: false,
                reason: Chosen
            }
        );
        // The user's ICED_BACKEND wins over the setting and is not overwritten.
        assert_eq!(
            d(Gpu, Some("tiny-skia"), None, true),
            Decision {
                set_iced_backend: None,
                gpu: false,
                reason: EnvIcedBackend
            }
        );
        assert_eq!(
            d(Cpu, Some("wgpu"), None, true),
            Decision {
                set_iced_backend: None,
                gpu: true,
                reason: EnvIcedBackend
            }
        );
        assert_eq!(
            d(Auto, Some(" "), None, true).reason,
            Detected,
            "an empty value is no override"
        );
        // K4_WATERFALL forces the waterfall path, but never the GPU path under software rendering
        // or without an adapter.
        assert_eq!(
            d(Auto, None, Some("cpu"), true),
            Decision {
                set_iced_backend: None,
                gpu: false,
                reason: EnvK4Waterfall
            }
        );
        assert_eq!(
            d(Auto, None, Some("gpu"), true),
            Decision {
                set_iced_backend: None,
                gpu: true,
                reason: EnvK4Waterfall
            }
        );
        assert!(
            !d(Auto, None, Some("gpu"), false).gpu,
            "no adapter, no GPU waterfall"
        );
        assert!(
            !d(Cpu, None, Some("gpu"), true).gpu,
            "software renderer: a shader draws nothing"
        );
        assert!(!d(Gpu, Some("tiny-skia"), Some("gpu"), true).gpu);
    }

    /// FR-UI-27: the status line says what draws and why; "GPU" only once a GPU frame was drawn.
    /// trace: FR-UI-27
    #[test]
    fn fr_ui_27_status_line() {
        let gpu = decide(Renderer::Auto, None, None, || true);
        assert_eq!(
            status_text(&gpu, true),
            "In use: GPU (wgpu) — a GPU was found."
        );
        assert!(status_text(&gpu, false).starts_with("GPU selected"));
        let cpu = decide(Renderer::Cpu, None, None, || true);
        assert_eq!(
            status_text(&cpu, true),
            "In use: CPU (software) — chosen in Settings."
        );
        let none = decide(Renderer::Gpu, None, None, || false);
        assert_eq!(
            status_text(&none, false),
            "In use: CPU (software) — GPU chosen, but none was found."
        );
    }
}
