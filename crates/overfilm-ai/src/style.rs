//! Editing style learnt from finished edits (a Premiere Pro sequence brought in as FCP XML, or
//! any sequence edited in the editor): how long shots and kept stretches run, how often and how
//! the cuts are dressed (transitions), which effects come up, how much of the footage is kept and
//! how captions are written. The suggestions then follow it (highlight length, joins).

use serde::{Deserialize, Serialize};

/// What one finished sequence shows; the engine reads it off the timeline.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EditFacts {
    pub length_ms: i64,
    /// Time between consecutive video edit points.
    pub shots_ms: Vec<i64>,
    /// Stretches of one source played on without a jump (consecutive clips that continue the
    /// source join into one).
    pub segments_ms: Vec<i64>,
    /// Video transitions: effect id and length.
    pub transitions: Vec<(String, i64)>,
    /// Video edit points (cuts) inside the sequence.
    pub cuts: usize,
    /// Effects added by the editor (not the built-in Motion / Opacity / Volume), one entry per use;
    /// "zoom" for a keyframed Motion scale or position.
    pub effects: Vec<String>,
    /// Length of the source media the kept parts came from (0 when unknown).
    pub sources_ms: i64,
    /// Length of the sources kept in the edit.
    pub kept_ms: i64,
    pub captions: Vec<CaptionFact>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CaptionFact {
    pub chars: usize,
    pub lines: usize,
    pub ms: i64,
}

/// Quartiles of a list (ms).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spread {
    pub p25: i64,
    pub median: i64,
    pub p75: i64,
    pub count: usize,
}

fn spread(v: &[i64]) -> Spread {
    let mut v: Vec<i64> = v.iter().copied().filter(|&x| x > 0).collect();
    if v.is_empty() {
        return Spread::default();
    }
    v.sort_unstable();
    let at = |q: usize| v.get((v.len() - 1) * q / 100).copied().unwrap_or(0);
    Spread { p25: at(25), median: at(50), p75: at(75), count: v.len() }
}

/// A learnt editing style.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Style {
    /// Sequences learnt from and their total length.
    pub sequences: usize,
    pub length_ms: i64,
    pub shot: Spread,
    pub segment: Spread,
    pub cuts_per_minute: f32,
    /// Share of cuts with a transition, the transitions' length and the most used ones.
    pub transition_share: f32,
    pub transition: Spread,
    pub transitions: Vec<(String, usize)>,
    /// Most used effects and how many uses per minute.
    pub effects: Vec<(String, f32)>,
    pub zooms_per_minute: f32,
    /// Kept share of the source footage (None when the sources' length is unknown).
    pub kept_share: Option<f32>,
    pub caption_chars: Spread,
    pub caption_ms: Spread,
    /// Share of two-line captions.
    pub two_line_share: f32,
}

fn top(names: impl Iterator<Item = String>, n: usize) -> Vec<(String, usize)> {
    let mut m: Vec<(String, usize)> = Vec::new();
    for x in names {
        match m.iter_mut().find(|(k, _)| *k == x) {
            Some(e) => e.1 += 1,
            None => m.push((x, 1)),
        }
    }
    m.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    m.truncate(n);
    m
}

/// Learn the style of finished edits.
pub fn learn(facts: &[EditFacts]) -> Style {
    let length_ms: i64 = facts.iter().map(|f| f.length_ms.max(0)).fold(0, i64::saturating_add);
    let minutes = (length_ms as f32 / 60_000.0).max(1e-3);
    let all = |g: fn(&EditFacts) -> &Vec<i64>| facts.iter().flat_map(|f| g(f).iter().copied()).collect::<Vec<i64>>();
    let cuts: usize = facts.iter().map(|f| f.cuts).fold(0, usize::saturating_add);
    let trans: Vec<&(String, i64)> = facts.iter().flat_map(|f| f.transitions.iter()).collect();
    let effects = top(facts.iter().flat_map(|f| f.effects.iter().filter(|e| *e != "zoom").cloned()), 8);
    let zooms = facts.iter().flat_map(|f| f.effects.iter()).filter(|e| *e == "zoom").count();
    let (src, kept) = facts.iter().filter(|f| f.sources_ms > 0).fold((0i64, 0i64), |(a, b), f| (a.saturating_add(f.sources_ms), b.saturating_add(f.kept_ms)));
    let caps: Vec<&CaptionFact> = facts.iter().flat_map(|f| f.captions.iter()).collect();
    Style {
        sequences: facts.len(),
        length_ms,
        shot: spread(&all(|f| &f.shots_ms)),
        segment: spread(&all(|f| &f.segments_ms)),
        cuts_per_minute: cuts as f32 / minutes,
        transition_share: if cuts == 0 { 0.0 } else { (trans.len() as f32 / cuts as f32).min(1.0) },
        transition: spread(&trans.iter().map(|t| t.1).collect::<Vec<_>>()),
        transitions: top(trans.iter().map(|t| t.0.clone()), 4),
        effects: effects.into_iter().map(|(n, c)| (n, c as f32 / minutes)).collect(),
        zooms_per_minute: zooms as f32 / minutes,
        kept_share: (src > 0).then(|| (kept as f32 / src as f32).clamp(0.0, 1.0)),
        caption_chars: spread(&caps.iter().map(|c| c.chars as i64).collect::<Vec<_>>()),
        caption_ms: spread(&caps.iter().map(|c| c.ms).collect::<Vec<_>>()),
        two_line_share: if caps.is_empty() { 0.0 } else { caps.iter().filter(|c| c.lines >= 2).count() as f32 / caps.len() as f32 },
    }
}

impl Style {
    /// Highlight length that matches how long the editor lets a moment run: the upper quartile of
    /// the kept stretches, 5–60 s. None before anything is learnt.
    pub fn highlight_length_ms(&self) -> Option<i64> {
        (self.segment.count >= 3).then(|| self.segment.p75.clamp(5_000, 60_000))
    }

    /// The join between moments: Some(length) of a dissolve when the editor dresses at least a
    /// fifth of the cuts, Some(0) for plain cuts, None before anything is learnt.
    pub fn join_ms(&self) -> Option<i64> {
        if self.shot.count < 3 {
            return None;
        }
        Some(if self.transition_share >= 0.2 { self.transition.median.clamp(100, 3_000) } else { 0 })
    }

    /// The style in a few Korean lines.
    pub fn summary_ko(&self) -> Vec<String> {
        let s = |ms: i64| format!("{:.1}초", ms as f64 / 1000.0);
        let mut out = vec![format!("시퀀스 {}개, 모두 {:.0}분", self.sequences, self.length_ms as f64 / 60_000.0)];
        if self.shot.count > 0 {
            out.push(format!("샷 길이 보통 {} ({}~{}), 분당 컷 {:.1}번", s(self.shot.median), s(self.shot.p25), s(self.shot.p75), self.cuts_per_minute));
        }
        if self.segment.count > 0 {
            out.push(format!("한 번에 이어서 쓰는 구간 보통 {} (길면 {})", s(self.segment.median), s(self.segment.p75)));
        }
        out.push(if self.transition_share >= 0.01 {
            let names: Vec<&str> = self.transitions.iter().map(|t| t.0.as_str()).collect();
            format!("컷의 {:.0}%에 전환 효과 ({}, 보통 {})", self.transition_share * 100.0, names.join(", "), s(self.transition.median))
        } else {
            "전환 효과 거의 없이 바로 컷".into()
        });
        if self.zooms_per_minute > 0.0 {
            out.push(format!("줌(확대 키프레임) 분당 {:.1}번", self.zooms_per_minute));
        }
        if !self.effects.is_empty() {
            let e: Vec<String> = self.effects.iter().take(4).map(|(n, r)| format!("{n} {r:.1}/분")).collect();
            out.push(format!("자주 쓰는 효과: {}", e.join(", ")));
        }
        if let Some(k) = self.kept_share {
            out.push(format!("원본에서 남기는 양 {:.0}%", k * 100.0));
        }
        if self.caption_chars.count > 0 {
            out.push(format!(
                "자막 한 개 보통 {}자, {} 동안, 두 줄 자막 {:.0}%",
                self.caption_chars.median,
                s(self.caption_ms.median),
                self.two_line_share * 100.0
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_rhythm_joins_and_captions() {
        let f = EditFacts {
            length_ms: 120_000,
            shots_ms: vec![2_000, 3_000, 4_000, 5_000, 6_000],
            segments_ms: vec![8_000, 10_000, 12_000, 30_000],
            transitions: vec![("cross_dissolve".into(), 500), ("cross_dissolve".into(), 700)],
            cuts: 4,
            effects: vec!["zoom".into(), "zoom".into(), "gaussian_blur".into()],
            sources_ms: 600_000,
            kept_ms: 120_000,
            captions: vec![CaptionFact { chars: 12, lines: 1, ms: 1500 }, CaptionFact { chars: 20, lines: 2, ms: 2500 }],
        };
        let st = learn(&[f]);
        assert_eq!(st.shot.median, 4_000);
        assert_eq!(st.cuts_per_minute, 2.0);
        assert_eq!(st.transition_share, 0.5);
        assert_eq!(st.transitions, vec![("cross_dissolve".to_string(), 2)]);
        assert_eq!(st.zooms_per_minute, 1.0);
        assert_eq!(st.effects, vec![("gaussian_blur".to_string(), 0.5)]);
        assert_eq!(st.kept_share, Some(0.2));
        assert_eq!(st.two_line_share, 0.5);
        assert_eq!(st.highlight_length_ms(), Some(12_000));
        assert_eq!(st.join_ms(), Some(500));
        assert!(st.summary_ko().iter().any(|l| l.contains("50%")), "{:?}", st.summary_ko());
    }

    #[test]
    fn nothing_learnt_suggests_nothing_and_hostile_numbers_are_fine() {
        let st = learn(&[]);
        assert_eq!(st.highlight_length_ms(), None);
        assert_eq!(st.join_ms(), None);
        assert!(!st.summary_ko().is_empty());
        let f = EditFacts { length_ms: i64::MAX, shots_ms: vec![-5, i64::MAX, 0, 1, 2], cuts: usize::MAX / 2, sources_ms: -1, ..Default::default() };
        let st = learn(&[f.clone(), f]);
        assert!(st.cuts_per_minute.is_finite());
        assert_eq!(st.kept_share, None);
        assert_eq!(st.join_ms(), Some(0), "no transitions: plain cuts");
    }
}
