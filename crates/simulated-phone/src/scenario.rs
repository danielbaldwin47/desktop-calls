//! Declarative scenario files: what the Phone does, one step per line.
//!
//! ```text
//! # The Phone comes into range and hands-free connects.
//! connect
//! ```
//!
//! Blank lines and lines starting with `#` are ignored. The steps are the
//! control interface's actions, run in order once the Simulated Phone is on
//! the bus:
//!
//! - `connect`: hands-free comes up, as `Connect` does;
//! - `disconnect`: hands-free drops, as `Disconnect` does.

/// One step of a scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Connect,
    Disconnect,
}

/// Reads a scenario file's text. The error names the first bad line.
pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let step = match line {
            "connect" => Step::Connect,
            "disconnect" => Step::Disconnect,
            _ => {
                return Err(format!(
                    "line {}: unknown step {line:?}; the steps are connect and disconnect",
                    index + 1
                ));
            }
        };
        steps.push(step);
    }
    Ok(steps)
}
