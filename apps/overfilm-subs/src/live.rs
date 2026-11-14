//! Recognition as it happens: each chunk's text while the recogniser writes it, for the 진행
//! page's "방금 인식한 말".

use overfilm_asr::LiveText;

/// One chunk of the input and its text so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveChunk {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    /// The recogniser is done with it (the text is corrected).
    pub done: bool,
}

/// Chunks kept at most (a chunk is up to a minute: well over a day of sound).
const MAX_CHUNKS: usize = 2000;
/// Characters kept per chunk (a minute of fast speech is under a thousand).
const MAX_TEXT: usize = 4000;

/// The chunks heard so far, in time order.
#[derive(Clone, Debug, Default)]
pub struct Live {
    chunks: Vec<LiveChunk>,
}

impl Live {
    /// Take a chunk's text: partial text grows its chunk, a done chunk is final (later partial
    /// text for it is stale and ignored).
    pub fn update(&mut self, t: &LiveText) {
        let text: String = t.text.trim().chars().take(MAX_TEXT).collect();
        match self.chunks.binary_search_by_key(&t.start_ms, |c| c.start_ms) {
            Ok(k) => {
                if let Some(c) = self.chunks.get_mut(k)
                    && (t.done || !c.done)
                {
                    c.end_ms = t.end_ms;
                    c.text = text;
                    c.done = t.done;
                }
            }
            Err(k) => {
                if self.chunks.len() >= MAX_CHUNKS {
                    return;
                }
                self.chunks.insert(k, LiveChunk { start_ms: t.start_ms, end_ms: t.end_ms, text, done: t.done });
            }
        }
    }

    /// The last `n` chunks that have text, in time order: every chunk still being written plus
    /// the latest finished ones.
    pub fn recent(&self, n: usize) -> Vec<&LiveChunk> {
        let with_text: Vec<&LiveChunk> = self.chunks.iter().filter(|c| !c.text.is_empty()).collect();
        let writing = with_text.iter().filter(|c| !c.done).count();
        let keep = n.max(writing);
        with_text.get(with_text.len().saturating_sub(keep)..).map(<[_]>::to_vec).unwrap_or_default()
    }

    /// How many chunks are finished.
    pub fn done(&self) -> usize {
        self.chunks.iter().filter(|c| c.done).count()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
    }
}

/// The end of `text`, at most `max` characters, cut at a space when there is one near
/// ("…" in front when cut): what is being heard right now.
pub fn tail(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    let rest: String = text.chars().skip(n - max).collect();
    let cut = rest.find(' ').filter(|&i| i < rest.len() / 3).map_or(rest.as_str(), |i| rest.get(i + 1..).unwrap_or(&rest));
    format!("…{cut}")
}

/// `m:ss` (or `h:mm:ss`) of a time in ms.
pub fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lt(start_ms: i64, text: &str, done: bool) -> LiveText<'_> {
        LiveText { start_ms, end_ms: start_ms + 60_000, text, done }
    }

    #[test]
    fn chunks_grow_in_time_order_and_finish() {
        let mut l = Live::default();
        l.update(&lt(60_000, "둘째", false));
        l.update(&lt(0, "첫", false));
        l.update(&lt(0, "첫째 조각", false));
        assert_eq!(l.recent(5).iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["첫째 조각", "둘째"]);
        l.update(&lt(0, "첫째 조각입니다", true));
        // stale partial text after the corrected one is ignored
        l.update(&lt(0, "첫째 조", false));
        assert_eq!(l.recent(5).first().map(|c| (c.text.as_str(), c.done)), Some(("첫째 조각입니다", true)));
        assert_eq!(l.done(), 1);
        l.clear();
        assert!(l.is_empty());
    }

    #[test]
    fn recent_keeps_every_chunk_being_written() {
        let mut l = Live::default();
        for k in 0..10 {
            l.update(&lt(k * 60_000, "말", k < 4));
        }
        // six still being written: all of them even when asked for three
        assert_eq!(l.recent(3).len(), 6);
        assert_eq!(l.recent(8).len(), 8);
        // empty text is not shown
        l.update(&lt(20 * 60_000, "  ", false));
        assert_eq!(l.recent(100).len(), 10);
    }

    #[test]
    fn tail_and_clock() {
        assert_eq!(tail("짧은 말", 10), "짧은 말");
        let t = tail("가나다 라마바 사아자 차카타 파하", 9);
        assert!(t.starts_with('…') && t.chars().count() <= 10, "{t}");
        assert_eq!(clock(75_500), "1:15");
        assert_eq!(clock(3_725_000), "1:02:05");
        assert_eq!(clock(-5), "0:00");
    }
}
