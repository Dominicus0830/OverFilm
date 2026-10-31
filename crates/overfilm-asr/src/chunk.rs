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

    #[test]
    fn short_and_empty() {
        assert_eq!(split(&[0.1; 10], 16_000, 30.0, 5.0), vec![(0, 10)]);
        assert_eq!(split(&[], 16_000, 30.0, 5.0), vec![(0, 0)]);
    }
}
