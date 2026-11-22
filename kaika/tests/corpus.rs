//! Tests driven by corpus/discovery.json: 600+ decisions about third-party apps (official site, right download)
//! with lookalikes, typosquats, nightly/beta/checksum/debug files, localized builds, and injections.
//! Everything here is deterministic; the real model runs over the same file in tests/real_model.rs (opt-in).

use kaika::decision::{decide, Cand, Decider, Pick, Task};
use kaika::discovery::fast_path;
use kaika::quarantine::is_hostile;
use kaika::outcome::Outcome;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

mod support;
use support::{corpus, links_of};

/// Records which candidate ids reached it, and gives no answer.
struct Recorder(RefCell<Vec<String>>);
impl Decider for Recorder {
    fn choose(&self, _: Task, _: &str, cands: &[Cand]) -> Outcome<Pick> {
        self.0.borrow_mut().extend(cands.iter().map(|c| c.id.clone()));
        Outcome::Unsure("recorder".into())
    }
}

#[test]
fn the_corpus_is_large_varied_and_well_formed() {
    let c = corpus();
    assert!(c.len() >= 600, "{} cases", c.len());
    let mut by: HashMap<&str, usize> = HashMap::new();
    for x in &c {
        *by.entry(x.category.as_str()).or_default() += 1;
    }
    assert!(by.len() >= 20, "{} categories", by.len());
    assert!(by.values().all(|n| *n >= 9), "every category has real weight: {by:?}");
    let names: HashSet<&str> = c.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names.len(), c.len(), "case names are unique");
    for x in &c {
        assert!(x.kind == "site" || x.kind == "download", "{}", x.name);
        let ids: Vec<&str> = x.candidates.iter().map(|k| k.id.as_str()).collect();
        assert!(ids.len() >= 2 && ids.len() <= 8, "{}", x.name);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len(), "{}: unique ids", x.name);
        if let Some(e) = &x.expected {
            assert!(ids.contains(&e.as_str()), "{}: expected {e} is not a candidate", x.name);
            assert!(!x.candidates.iter().find(|k| &k.id == e).unwrap().hostile, "{}: the right answer cannot be hostile", x.name);
        }
        if x.kind == "download" {
            assert!(x.candidates.iter().all(|k| k.file.is_some()), "{}: download candidates name their file", x.name);
        }
    }
    assert!(c.iter().filter(|x| x.expected.is_none()).count() >= 30 && c.iter().filter(|x| x.injection).count() >= 100);
}

#[test]
fn the_quarantine_flags_exactly_the_marked_candidates_nothing_missed_nothing_extra() {
    let (mut flagged, mut clean) = (0, 0);
    for x in corpus() {
        for k in &x.candidates {
            assert_eq!(is_hostile(&k.text), k.hostile, "{}: {:?}", x.name, k.text);
            if k.hostile { flagged += 1 } else { clean += 1 }
        }
    }
    assert!(flagged >= 80, "{flagged} hostile candidates checked");
    assert!(clean >= 1000, "{clean} clean candidates checked: no false positives");
}

#[test]
fn farm_addresses_are_flagged_exactly_and_no_official_vendor_is_ever_caught() {
    let (mut farms, mut sites) = (0, 0);
    for x in corpus().iter().filter(|x| x.kind == "site") {
        for k in &x.candidates {
            assert_eq!(kaika::quarantine::is_farm_host(&k.id), k.farm, "{}: {}", x.name, k.id);
            sites += 1;
            if k.farm { farms += 1 }
            if x.expected.as_deref() == Some(k.id.as_str()) {
                assert!(!k.farm && !kaika::quarantine::is_farm_host(&k.id), "{}: the official site {} must never be dropped", x.name, k.id);
            }
        }
    }
    assert!(farms >= 60 && sites >= 700, "{farms} farm addresses among {sites} site candidates");
}

#[test]
fn a_hostile_candidate_never_reaches_the_decider_in_any_case() {
    for x in corpus() {
        let rec = Recorder(Default::default());
        let cands = x.candidates.iter().map(|k| Cand::new(k.id.clone(), k.text.clone())).collect();
        let judged = decide(&rec, if x.kind == "site" { Task::Site } else { Task::Download }, &x.query, cands);
        let hostile: Vec<&str> = x.candidates.iter().filter(|k| k.hostile).map(|k| k.id.as_str()).collect();
        for id in rec.0.borrow().iter() {
            assert!(!hostile.contains(&id.as_str()), "{}: {id} reached the model", x.name);
        }
        // dropped = hostile text, plus (for sites only) download-farm addresses
        let ignored = x.candidates.iter().filter(|k| k.hostile || (x.kind == "site" && k.farm)).count();
        assert_eq!(judged.dropped.len(), ignored, "{}", x.name);
        // and a hostile candidate is never the answer, whatever the decider does
        if let Outcome::Picked(p) = judged.outcome {
            assert!(!hostile.contains(&p.id.as_str()), "{}: picked a hostile candidate", x.name);
        }
    }
}

#[test]
fn the_deterministic_download_shortcut_is_never_wrong_and_settles_a_good_share() {
    let (mut fired, mut total) = (0, 0);
    let mut wrong = vec![];
    for x in corpus().iter().filter(|x| x.kind == "download") {
        total += 1;
        let links = links_of(x);
        if let Some(i) = fast_path(&links) {
            fired += 1;
            if Some(&links[i].1) != x.expected.as_ref() {
                wrong.push(format!("{}: took {} but expected {:?} (scores {:?})", x.name, links[i].1, x.expected, links.iter().map(|l| l.0).collect::<Vec<_>>()));
            }
        }
    }
    assert!(wrong.is_empty(), "the shortcut chose wrongly in {} cases:\n{}", wrong.len(), wrong.join("\n"));
    assert!(fired * 100 >= total * 30, "the shortcut settled only {fired} of {total} download cases");
    eprintln!("deterministic shortcut settled {fired} of {total} download decisions without any model");
}

#[test]
fn no_answer_download_cases_have_nothing_the_shortcut_could_take() {
    for x in corpus().iter().filter(|x| x.kind == "download" && x.expected.is_none()) {
        assert!(links_of(x).is_empty(), "{}: a non-mac candidate scored as installable", x.name);
    }
}

/// Real vendor pages (corpus/real.json): well formed, and the deterministic shortcut checked on them.
/// The real pages are not a gate for the model; this only pins what the shortcut does on them.
#[test]
fn the_real_corpus_is_well_formed() {
    let c = support::real_corpus();
    assert!(c.len() >= 30, "{} real cases", c.len());
    assert!(c.iter().filter(|x| x.expected.is_none()).count() >= 10, "at least 10 no-answer cases");
    let names: HashSet<&str> = c.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names.len(), c.len(), "case names are unique");
    for x in &c {
        assert_eq!(x.kind, "download", "{}: real cases are download pages", x.name);
        assert!(!x.injection, "{}: no real page is an injection test", x.name);
        let ids: Vec<&str> = x.candidates.iter().map(|k| k.id.as_str()).collect();
        assert!(ids.len() >= 2 && ids.len() <= 8, "{}", x.name);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len(), "{}: unique ids", x.name);
        assert!(ids.iter().all(|id| id.starts_with("https://")), "{}: every candidate is an https URL", x.name);
        for k in &x.candidates {
            assert!(k.file.is_some(), "{}: {} names its file", x.name, k.id);
            assert!(!k.hostile && !k.farm, "{}: a real page's candidate is not flagged: {}", x.name, k.id);
            assert_eq!(is_hostile(&k.text), false, "{}: the quarantine must not flag real text: {:?}", x.name, k.text);
        }
        if let Some(e) = &x.expected {
            assert!(ids.contains(&e.as_str()), "{}: expected {e} is not a candidate", x.name);
        }
    }
}

/// Real-page cases where the deterministic shortcut answers and is wrong. Recorded, not accepted: the list is
/// pinned exactly, so a new miss fails this test and a fixed one must be removed from the list.
const KNOWN_SHORTCUT_MISSES: &[&str] = &[
    "no-answer: Rufus (Windows only)",
    "no-answer: Everything search (Windows only)",
    "no-answer: Sumatra PDF (Windows only)",
    "no-answer: PuTTY (no macOS build on the page)",
    "no-answer: AutoHotkey v2 (Windows only)",
];

#[test]
fn the_deterministic_shortcut_on_real_pages_is_never_wrong_except_the_recorded_misses() {
    let mut misses: Vec<String> = vec![];
    for x in support::real_corpus() {
        let links = links_of(&x);
        if let Some(i) = fast_path(&links) {
            if Some(&links[i].1) != x.expected.as_ref() {
                misses.push(x.name.clone());
            }
        }
    }
    let known: Vec<String> = KNOWN_SHORTCUT_MISSES.iter().map(|s| s.to_string()).collect();
    assert_eq!(misses, known, "shortcut misses on real pages changed");
}
