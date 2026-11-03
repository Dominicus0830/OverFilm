# overfilm-eval samples

Tiny hand-made transcripts that show what the tool reports and pin its numbers in tests
(`lib.rs`, `sample_pair_scores`). They are not the evaluation set: the real reference videos and
captions (M1) stay outside the repository.

```sh
cargo run -p overfilm-eval -- --terms crates/overfilm-eval/samples/terms.txt \
  crates/overfilm-eval/samples/ref crates/overfilm-eval/samples/hyp
```

| file | what it shows | hand count |
|---|---|---|
| `clip01` | typical recognition errors: `아나` → `안아`, `솔저` → `솔져`, `위도우메이커` split in two | 38 chars (`76` reads `칠십육`), 3 substituted (CER 7.89%); 16 words, 4 errors (WER 25%); terms 4 of 6 (`아나`, `솔저: 76` missed) |
| `clip02` | the same words with different spacing and punctuation (`.srt` reference, `.txt` hypothesis) | 23 chars, 0 errors (CER 0%); 7 words, 3 errors (WER 42.86%), which is why CER is the headline number for Korean; terms 2 of 2 |
| `clip03` | a reference with no hypothesis | listed under "no hypothesis for" |

Totals: CER 3/61 = 4.92%, WER 7/23 = 30.43%, terms 6/8 = 75%.
