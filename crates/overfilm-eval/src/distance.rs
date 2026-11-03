//! Levenshtein alignment counts (substitutions, deletions, insertions, hits).

use serde::Serialize;

/// The largest reference × hypothesis table [`align`] will fill. Two hours of Korean speech is
/// about 40 000 characters a side; anything longer should be scored per file or per segment.
pub const MAX_CELLS: u64 = 2_000_000_000;

/// The longest side [`align`] accepts (it allocates rows of this length).
pub const MAX_LEN: usize = 1_000_000;

/// Edit operations of a minimum-cost alignment of a hypothesis to a reference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EditCounts {
    /// Reference length.
    pub reference: usize,
    /// Hypothesis length.
    pub hypothesis: usize,
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
    pub hits: usize,
}

impl EditCounts {
    /// Total edits.
    pub fn errors(&self) -> usize {
        self.substitutions.saturating_add(self.deletions).saturating_add(self.insertions)
    }
    /// Error rate: edits ÷ reference length. An empty reference scores 0 against an empty
    /// hypothesis and 1 otherwise (every hypothesis item is an insertion).
    pub fn rate(&self) -> f64 {
        if self.reference == 0 {
            return if self.hypothesis == 0 { 0.0 } else { 1.0 };
        }
        self.errors() as f64 / self.reference as f64
    }
    /// Accumulate another alignment (corpus-level rates are total edits ÷ total reference).
    pub fn add(&mut self, o: &EditCounts) {
        self.reference = self.reference.saturating_add(o.reference);
        self.hypothesis = self.hypothesis.saturating_add(o.hypothesis);
        self.substitutions = self.substitutions.saturating_add(o.substitutions);
        self.deletions = self.deletions.saturating_add(o.deletions);
        self.insertions = self.insertions.saturating_add(o.insertions);
        self.hits = self.hits.saturating_add(o.hits);
    }
}

/// Inputs too long to align in one table.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reference} × {hypothesis} items is too long to align at once (limits: {MAX_LEN} a side, {MAX_CELLS} cells); score shorter files")]
pub struct TooLong {
    pub reference: usize,
    pub hypothesis: usize,
}

#[derive(Clone, Copy, Default)]
struct Cell {
    cost: usize,
    s: usize,
    d: usize,
    i: usize,
}

impl Cell {
    fn step(self, s: usize, d: usize, i: usize) -> Cell {
        Cell { cost: self.cost + s + d + i, s: self.s + s, d: self.d + d, i: self.i + i }
    }
}

/// Align `hyp` to `reference` with unit costs and count the operations. Ties prefer a hit or a
/// substitution, then a deletion, then an insertion, so counts are deterministic. Memory is two
/// rows; time is `reference.len() × hyp.len()`, capped at [`MAX_CELLS`].
pub fn align<T: PartialEq>(reference: &[T], hyp: &[T]) -> Result<EditCounts, TooLong> {
    let (n, m) = (reference.len(), hyp.len());
    if n > MAX_LEN || m > MAX_LEN || (n as u64).saturating_mul(m as u64) > MAX_CELLS {
        return Err(TooLong { reference: n, hypothesis: m });
    }
    // prev[j]: cost of aligning reference[..i] with hyp[..j].
    let mut prev: Vec<Cell> = (0..=m).map(|j| Cell { cost: j, i: j, ..Cell::default() }).collect();
    let mut cur: Vec<Cell> = vec![Cell::default(); m + 1];
    for r in reference {
        if let (Some(c0), Some(p0)) = (cur.first_mut(), prev.first()) {
            *c0 = p0.step(0, 1, 0);
        }
        for (j, h) in hyp.iter().enumerate() {
            let (Some(&diag), Some(&up), Some(&left)) = (prev.get(j), prev.get(j + 1), cur.get(j)) else { continue };
            let sub = diag.step(usize::from(r != h), 0, 0);
            let del = up.step(0, 1, 0);
            let ins = left.step(0, 0, 1);
            let mut best = sub;
            if del.cost < best.cost {
                best = del;
            }
            if ins.cost < best.cost {
                best = ins;
            }
            if let Some(slot) = cur.get_mut(j + 1) {
                *slot = best;
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let last = prev.last().copied().unwrap_or_default();
    let hits = n.saturating_sub(last.s).saturating_sub(last.d);
    Ok(EditCounts { reference: n, hypothesis: m, substitutions: last.s, deletions: last.d, insertions: last.i, hits })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn identical() {
        let c = align(&chars("라인하르트"), &chars("라인하르트")).unwrap();
        assert_eq!((c.hits, c.errors()), (5, 0));
        assert_eq!(c.rate(), 0.0);
    }

    #[test]
    fn substitution_deletion_insertion() {
        // 젠야타 -> 제냐타: two substitutions.
        let c = align(&chars("젠야타"), &chars("제냐타")).unwrap();
        assert_eq!((c.substitutions, c.deletions, c.insertions, c.hits), (2, 0, 0, 1));
        let c = align(&chars("abcd"), &chars("abd")).unwrap();
        assert_eq!((c.substitutions, c.deletions, c.insertions), (0, 1, 0));
        let c = align(&chars("abd"), &chars("abcd")).unwrap();
        assert_eq!((c.substitutions, c.deletions, c.insertions), (0, 0, 1));
    }

    #[test]
    fn empty_sides() {
        let c = align::<char>(&[], &[]).unwrap();
        assert_eq!(c.rate(), 0.0);
        let c = align(&[], &chars("ab")).unwrap();
        assert_eq!((c.insertions, c.rate()), (2, 1.0));
        let c = align(&chars("ab"), &[]).unwrap();
        assert_eq!((c.deletions, c.rate()), (2, 1.0));
    }

    #[test]
    fn rate_can_exceed_one() {
        let c = align(&chars("a"), &chars("bcd")).unwrap();
        assert_eq!(c.errors(), 3);
        assert_eq!(c.rate(), 3.0);
    }

    #[test]
    fn too_long_is_an_error_not_a_hang() {
        let big = vec![0u8; 100_000];
        assert!(align(&big, &big).is_err());
        let huge = vec![0u8; MAX_LEN + 1];
        assert!(align(&[], &huge).is_err());
    }
}
