//! Is there a network at all? Asked before anything slow starts, so an offline run fails in about two
//! seconds instead of after a model has loaded or a solver has run.

use crate::runner::{Runner, argv};

/// What the check connects to: the same service discovery depends on.
pub const PROBE: &str = "https://www.wikidata.org/";

/// `KAIKA_OFFLINE=1` forces offline (for scripts and tests).
pub fn online(r: &dyn Runner) -> bool {
    if std::env::var("KAIKA_OFFLINE").as_deref() == Ok("1") {
        return false;
    }
    r.run(&argv(&[
        "curl",
        "-s",
        "-o",
        "/dev/null",
        "-I",
        "--connect-timeout",
        "2",
        "--max-time",
        "3",
        PROBE,
    ]))
    .map(|o| o.ok())
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Fake;

    const CMD: &str =
        "curl -s -o /dev/null -I --connect-timeout 2 --max-time 3 https://www.wikidata.org/";

    #[test]
    fn online_when_the_probe_answers() {
        assert!(online(&Fake::default().with(CMD, "")));
    }

    #[test]
    fn offline_when_it_does_not() {
        assert!(!online(&Fake::default().failing(CMD)));
        assert!(
            !online(&Fake::default()),
            "a missing curl or a refused connection reads as offline"
        );
    }
}
