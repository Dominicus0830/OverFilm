//! Cutting long audio into chunks the models take: each cut lands on the quietest 100 ms in the
//! last `search` seconds before the length limit, so chunks never exceed the limit and rarely
//! split a word. Chunks cover the input exactly (no gaps, no overlap).

/// Sample ranges `[start, end)` of the chunks of `audio`.
pub fn split(audio: &[f32], sr: usize, max_sec: f32, search_sec: f32) -> Vec<(usize, usize)> {
    let n = audio.len();
    let max_len = ((max_sec.max(1.0) * sr as f32) as usize).max(1);
    let search = ((search_sec.max(0.0) * sr as f32) as usize).min(max_len / 2);
    let win = (sr / 10).max(1);
    // prefix sums of energy for O(1) window sums
    let mut pre = Vec::with_capacity(n + 1);
    pre.push(0f64);
    let mut acc = 0f64;
    for &x in audio {
        acc += f64::from(x) * f64::from(x);
        pre.push(acc);
    }
    let energy = |a: usize, b: usize| pre.get(b).copied().unwrap_or(acc) - pre.get(a).copied().unwrap_or(0.0);
    let mut out = Vec::new();
    let mut start = 0;
    while n - start > max_len {
        let limit = start + max_len;
        let lo = limit.saturating_sub(search).max(start + 1);
        let mut best = (f64::INFINITY, limit);
        let mut p = lo;
        while p + win <= limit {
            let e = energy(p, p + win);
            if e < best.0 {
                best = (e, p + win / 2);
            }
            p += win / 2;
        }
        let cut = best.1.clamp(start + 1, limit);
        out.push((start, cut));
        start = cut;
    }
    if start < n || out.is_empty() {
        out.push((start, n));
    }
    out
}

/// Shortest quiet stretch [`split_voiced`] leaves out, in seconds (shorter pauses stay inside a
/// piece so the recogniser keeps the sentence around them).
pub const MIN_SILENCE_SEC: f32 = 2.0;
/// Sound kept on each side of a left-out stretch, in seconds (the ends of words fade out).
pub const SILENCE_PAD_SEC: f32 = 0.3;
/// A 30 ms frame counts as quiet below this level (dBFS) …
const QUIET_FLOOR_DB: f32 = -55.0;
/// … or this far below the loud frames (the 95th percentile), whichever is higher.
const QUIET_BELOW_LOUD_DB: f32 = 40.0;

/// Sample ranges with sound in them: `audio` without its quiet stretches of at least `min_sec`
/// seconds (keeping `pad_sec` at each side). A frame is quiet when its level is below
/// −55 dBFS or 40 dB under the input's loud frames, whichever is higher, so steady room or
/// game noise far under the voice counts as quiet but a soft voice does not. Empty when the
/// whole input is quiet.
pub fn voiced(audio: &[f32], sr: usize, min_sec: f32, pad_sec: f32) -> Vec<(usize, usize)> {
    let frame = (sr * 3 / 100).max(1);
    let n = audio.len();
    let db: Vec<f32> = audio
        .chunks(frame)
        .map(|c| {
            let e = c.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / c.len().max(1) as f64;
            (10.0 * e.max(1e-12).log10()) as f32
        })
        .collect();
    if db.is_empty() {
        return Vec::new();
    }
    let mut sorted = db.clone();
    sorted.sort_by(f32::total_cmp);
    let loud = sorted.get(sorted.len() * 95 / 100).copied().unwrap_or(QUIET_FLOOR_DB);
    let threshold = QUIET_FLOOR_DB.max(loud - QUIET_BELOW_LOUD_DB);
    let min_frames = ((min_sec.max(0.0) * sr as f32) as usize / frame).max(1);
    let pad = (pad_sec.max(0.0) * sr as f32) as usize;
    // runs of loud frames, joined across quiet runs shorter than `min_frames`
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < db.len() {
        if db.get(i).is_some_and(|&d| d < threshold) {
            i += 1;
            continue;
        }
        let start = i;
        while i < db.len() && db.get(i).is_some_and(|&d| d >= threshold) {
            i += 1;
        }
        let (a, b) = ((start * frame).saturating_sub(pad), (i * frame).saturating_add(pad).min(n));
        match out.last_mut() {
            Some(last) if a <= last.1 || start.saturating_sub(last.1.saturating_sub(pad) / frame) < min_frames => last.1 = b,
            _ => out.push((a, b)),
        }
    }
    out
}

/// [`split`] of only the parts of `audio` with sound in them ([`voiced`]): pieces never span a
/// quiet stretch of `min_silence_sec` or more, which the recogniser then never sees. Empty when
/// the whole input is quiet.
pub fn split_voiced(audio: &[f32], sr: usize, max_sec: f32, search_sec: f32, min_silence_sec: f32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (a, b) in voiced(audio, sr, min_silence_sec, SILENCE_PAD_SEC) {
        let part = audio.get(a..b).unwrap_or(&[]);
        out.extend(split(part, sr, max_sec, search_sec).into_iter().filter(|(x, y)| y > x).map(|(x, y)| (a + x, a + y)));
    }
    out
}

/// Seconds of `audio` that [`split_voiced`] leaves out.
pub fn skipped_seconds(spans: &[(usize, usize)], total: usize, sr: usize) -> f32 {
    let kept: usize = spans.iter().map(|(a, b)| b.saturating_sub(*a)).sum();
    total.saturating_sub(kept) as f32 / sr.max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_input_and_respects_limit() {
        let sr = 100;
        // loud everywhere except a quiet gap at 27–28 s
        let audio: Vec<f32> = (0..sr * 70).map(|i| if (2700..2800).contains(&i) { 0.0 } else { 0.5 }).collect();
        let c = split(&audio, sr, 30.0, 5.0);
        assert_eq!(c.first().map(|x| x.0), Some(0));
        assert_eq!(c.last().map(|x| x.1), Some(audio.len()));
        assert!(c.windows(2).all(|w| w[0].1 == w[1].0));
        assert!(c.iter().all(|(a, b)| b - a <= 30 * sr && b > a));
        assert!((2700..2800).contains(&c[0].1), "{c:?}");
    }

    fn tone(sr: usize, secs: f32, amp: f32) -> Vec<f32> {
        (0..(secs * sr as f32) as usize).map(|i| amp * (i as f32 * 0.3).sin()).collect()
    }

    #[test]
    fn long_quiet_stretches_are_left_out() {
        let sr = 16_000;
        // 3 s speech, 5 s near-silence (noise 50 dB down), 2 s soft speech, 1 s pause, 2 s speech
        let mut a = tone(sr, 3.0, 0.3);
        a.extend(tone(sr, 5.0, 0.001));
        a.extend(tone(sr, 2.0, 0.03));
        a.extend(tone(sr, 1.0, 0.0));
        a.extend(tone(sr, 2.0, 0.3));
        let v = voiced(&a, sr, MIN_SILENCE_SEC, SILENCE_PAD_SEC);
        assert_eq!(v.len(), 2, "{v:?}");
        let s = |x: usize| x as f32 / sr as f32;
        assert!(s(v[0].0) == 0.0 && (3.2..3.4).contains(&s(v[0].1)), "{v:?}");
        // the soft voice and the short pause stay
        assert!((7.6..7.8).contains(&s(v[1].0)) && v[1].1 == a.len(), "{v:?}");
        let spans = split_voiced(&a, sr, 60.0, 5.0, MIN_SILENCE_SEC);
        assert_eq!(spans, v);
        assert!((4.0..4.6).contains(&skipped_seconds(&spans, a.len(), sr)));
        // long voiced stretches are still cut to the length limit
        let long = tone(sr, 130.0, 0.3);
        let spans = split_voiced(&long, sr, 60.0, 5.0, MIN_SILENCE_SEC);
        assert_eq!(spans.len(), 3);
        assert!(spans.iter().all(|(a, b)| b - a <= 60 * sr));
    }

    #[test]
    fn all_quiet_or_empty_gives_nothing() {
        assert!(voiced(&[], 16_000, 2.0, 0.3).is_empty());
        assert!(split_voiced(&vec![0.0; 16_000 * 5], 16_000, 60.0, 5.0, 2.0).is_empty());
        // one loud click is kept, not dropped
        let mut a = vec![0.0f32; 16_000 * 5];
        a[40_000] = 0.9;
        assert_eq!(voiced(&a, 16_000, 2.0, 0.3).len(), 1);
    }

    #[test]
    fn short_and_empty() {
        assert_eq!(split(&[0.1; 10], 16_000, 30.0, 5.0), vec![(0, 10)]);
        assert_eq!(split(&[], 16_000, 30.0, 5.0), vec![(0, 0)]);
    }
}
