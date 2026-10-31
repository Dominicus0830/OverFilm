//! Qwen2 byte-level BPE tokenizer (`vocab.json` + `merges.txt` + the added tokens of
//! `tokenizer_config.json`), written from the published format: NFC normalisation, the Qwen2
//! pre-tokenizer pattern, GPT-2's byte-to-unicode table, rank-ordered merges. Added tokens
//! (`<|im_start|>`, `<|audio_pad|>`, `<asr_text>`, `<timestamp>`…) match whole before BPE.

use std::collections::HashMap;

use fancy_regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::AsrError;

/// The Qwen2 pre-tokenizer pattern (`tokenizer.json` of the Qwen2 family).
const PATTERN: &str = r"(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\r\n\p{L}\p{N}]?\p{L}+|\p{N}| ?[^\s\p{L}\p{N}]+[\r\n]*|\s*[\r\n]+|\s+(?!\S)|\s+";

/// Longest piece BPE runs on; longer pieces (pathological input) are split into bytes.
const MAX_PIECE: usize = 256;

pub struct Tokenizer {
    vocab: HashMap<String, u32>,
    rev: HashMap<u32, String>,
    ranks: HashMap<(String, String), usize>,
    /// added tokens, longest first (so `<|audio_start|>` wins over a prefix)
    added: Vec<(String, u32)>,
    added_rev: HashMap<u32, String>,
    byte_enc: [char; 256],
    byte_dec: HashMap<char, u8>,
    re: Regex,
}

/// GPT-2's reversible byte → printable character table.
fn bytes_to_unicode() -> [char; 256] {
    let mut table = ['\0'; 256];
    let mut n = 0u32;
    for b in 0..=255u32 {
        let printable = (33..=126).contains(&b) || (161..=172).contains(&b) || (174..=255).contains(&b);
        let c = if printable {
            b
        } else {
            n += 1;
            255 + n
        };
        if let (Some(slot), Some(ch)) = (table.get_mut(b as usize), char::from_u32(c)) {
            *slot = ch;
        }
    }
    table
}

impl Tokenizer {
    /// Build from the three files' contents.
    pub fn new(vocab_json: &str, merges_txt: &str, tokenizer_config_json: &str) -> Result<Self, AsrError> {
        let vocab: HashMap<String, u32> = serde_json::from_str(vocab_json).map_err(|e| AsrError::Model(format!("vocab.json: {e}")))?;
        let mut ranks = HashMap::new();
        for line in merges_txt.lines() {
            if line.starts_with("#version") || line.trim().is_empty() {
                continue;
            }
            if let Some((a, b)) = line.split_once(' ') {
                let r = ranks.len();
                ranks.entry((a.to_string(), b.to_string())).or_insert(r);
            }
        }
        let cfg: serde_json::Value = serde_json::from_str(tokenizer_config_json).map_err(|e| AsrError::Model(format!("tokenizer_config.json: {e}")))?;
        let mut added = Vec::new();
        if let Some(map) = cfg.get("added_tokens_decoder").and_then(|v| v.as_object()) {
            for (id, tok) in map {
                if let (Ok(id), Some(content)) = (id.parse::<u32>(), tok.get("content").and_then(|c| c.as_str())) {
                    added.push((content.to_string(), id));
                }
            }
        }
        added.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.1.cmp(&b.1)));
        let added_rev = added.iter().map(|(s, i)| (*i, s.clone())).collect();
        let rev = vocab.iter().map(|(s, i)| (*i, s.clone())).collect();
        let byte_enc = bytes_to_unicode();
        let byte_dec = byte_enc.iter().enumerate().map(|(b, c)| (*c, b as u8)).collect();
        let re = Regex::new(PATTERN).map_err(|e| AsrError::Model(format!("tokenizer pattern: {e}")))?;
        Ok(Self { vocab, rev, ranks, added, added_rev, byte_enc, byte_dec, re })
    }

    /// The id of an added token such as `<|im_end|>`.
    pub fn special(&self, content: &str) -> Option<u32> {
        self.added.iter().find(|(s, _)| s == content).map(|(_, i)| *i)
    }

    /// Encode text; added tokens inside it are matched whole.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let mut out = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            match self.next_added(rest) {
                Some((pos, content, id)) => {
                    self.encode_plain(&rest[..pos], &mut out);
                    out.push(id);
                    rest = &rest[pos + content.len()..];
                }
                None => {
                    self.encode_plain(rest, &mut out);
                    break;
                }
            }
        }
        out
    }

    /// The earliest added token in `s` (longest at that position).
    fn next_added<'a>(&'a self, s: &str) -> Option<(usize, &'a str, u32)> {
        let mut best: Option<(usize, &str, u32)> = None;
        for (content, id) in &self.added {
            if let Some(p) = s.find(content.as_str())
                && best.is_none_or(|(bp, bc, _)| p < bp || (p == bp && content.len() > bc.len()))
            {
                best = Some((p, content.as_str(), *id));
            }
        }
        best
    }

    fn encode_plain(&self, text: &str, out: &mut Vec<u32>) {
        if text.is_empty() {
            return;
        }
        let norm: String = text.nfc().collect();
        for m in self.re.find_iter(&norm).flatten() {
            let mapped: String = m.as_str().bytes().filter_map(|b| self.byte_enc.get(b as usize).copied()).collect();
            if let Some(&id) = self.vocab.get(&mapped) {
                out.push(id);
                continue;
            }
            let symbols: Vec<String> = mapped.chars().map(String::from).collect();
            let merged = if symbols.len() > MAX_PIECE { symbols } else { self.bpe(symbols) };
            for s in merged {
                if let Some(&id) = self.vocab.get(&s) {
                    out.push(id);
                } else {
                    // every single byte symbol is in a byte-level vocabulary; skip anything else
                    out.extend(s.chars().filter_map(|c| self.vocab.get(&c.to_string()).copied()));
                }
            }
        }
    }

    /// Merge adjacent symbols lowest rank first until no pair is in the merge table.
    fn bpe(&self, mut symbols: Vec<String>) -> Vec<String> {
        loop {
            let mut best: Option<(usize, usize)> = None;
            for i in 0..symbols.len().saturating_sub(1) {
                let (Some(a), Some(b)) = (symbols.get(i), symbols.get(i + 1)) else { break };
                if let Some(&r) = self.ranks.get(&(a.clone(), b.clone()))
                    && best.is_none_or(|(_, br)| r < br)
                {
                    best = Some((i, r));
                }
            }
            let Some((i, _)) = best else { return symbols };
            let next = symbols.get(i + 1).cloned().unwrap_or_default();
            if let Some(s) = symbols.get_mut(i) {
                s.push_str(&next);
            }
            if i + 1 < symbols.len() {
                symbols.remove(i + 1);
            }
        }
    }

    /// Decode ids to text, dropping added tokens when `skip_added`.
    pub fn decode(&self, ids: &[u32], skip_added: bool) -> String {
        let mut bytes = Vec::new();
        for id in ids {
            if let Some(s) = self.added_rev.get(id) {
                if !skip_added {
                    bytes.extend_from_slice(s.as_bytes());
                }
                continue;
            }
            if let Some(tok) = self.rev.get(id) {
                bytes.extend(tok.chars().filter_map(|c| self.byte_dec.get(&c).copied()));
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> Tokenizer {
        // bytes "a", "b", "ab" and the merge a+b; Ġ (U+0120) is the space byte
        let vocab = r#"{"a":0,"b":1,"ab":2,"Ġ":3,"Ġa":4}"#;
        let merges = "#version: 0.2\na b\nĠ a\n";
        let cfg = r#"{"added_tokens_decoder":{"10":{"content":"<|im_end|>"},"11":{"content":"<x>"}}}"#;
        Tokenizer::new(vocab, merges, cfg).unwrap()
    }

    #[test]
    fn merges_and_added_tokens() {
        let t = tiny();
        assert_eq!(t.encode("ab<|im_end|> a"), vec![2, 10, 4]);
        assert_eq!(t.decode(&[2, 10, 4], false), "ab<|im_end|> a");
        assert_eq!(t.decode(&[2, 10, 4], true), "ab a");
        assert_eq!(t.special("<x>"), Some(11));
    }

    #[test]
    fn byte_table_is_a_bijection() {
        let t = bytes_to_unicode();
        let mut v: Vec<char> = t.to_vec();
        v.sort_unstable();
        v.dedup();
        assert_eq!(v.len(), 256);
        assert_eq!(t[b' ' as usize], 'Ġ');
    }

    #[test]
    fn hostile_input_does_not_panic() {
        let t = tiny();
        let long = "x".repeat(10_000);
        let _ = t.encode(&long);
        let _ = t.decode(&[u32::MAX, 999], false);
    }
}
