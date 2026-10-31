//! `cargo run -p overfilm-glossary --example correct -- <glossary dir> < text`: print each
//! correction the glossary and the Korean rules make, one per line (for checking rules on a
//! transcript).

use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).ok_or("usage: correct <glossary dir> < text")?;
    let mut g = overfilm_glossary::Glossary::default();
    let mut files: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "tsv")).collect();
    files.sort();
    for f in files {
        let cat = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        g.add_table(&cat, &std::fs::read_to_string(&f)?)?;
    }
    let c = overfilm_glossary::Corrector::new(&g).fuzzy(&g).korean().digits(true);
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    for line in text.lines() {
        for (from, to) in c.apply(line).changes {
            println!("{from}\t→\t{to}");
        }
    }
    Ok(())
}
