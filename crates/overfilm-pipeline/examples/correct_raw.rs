//! Correct the pure recognition kept by `recognise_raw` again, with or without a match prior
//! (evaluation tool): prints each chunk's corrected text, one per line.
//!
//! ```sh
//! cargo run -p overfilm-pipeline --example correct_raw -- run.jsonl [prior.json]
//! ```

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let text = std::fs::read_to_string(a.get(1).cloned().unwrap_or_default()).unwrap_or_default();
    let prior: Option<overfilm_pipeline::game::MatchPrior> = a.get(2).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok());
    let Ok(g) = overfilm_pipeline::pack::glossary(None, true) else { return eprintln!("glossary") };
    let c = overfilm_pipeline::pack::corrector(&g);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let raw = v["raw"].as_str().unwrap_or_default();
        let (s, e) = (v["start"].as_i64().unwrap_or(0), v["end"].as_i64().unwrap_or(0));
        let out = match &prior {
            Some(p) => c.apply_preferring(raw, &p.preferred(s, e)),
            None => c.apply(raw),
        };
        println!("{}", out.text);
    }
}
