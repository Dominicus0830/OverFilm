use super::*;

const FPS: f64 = 2.0;

/// Samples at 2 per second from (seconds, label) runs; label: '.' play, 'S' standby, 'P' setup,
/// 'Z' pause notice, 'F' frozen play, 'L' live HUD, 'G' frozen with HUD.
fn build(runs: &[(f64, char)]) -> Vec<Sample> {
    let mut v = Vec::new();
    let mut t = 0.0;
    for (len, c) in runs {
        let n = (len * FPS).round() as usize;
        for _ in 0..n {
            let (screen, still) = match c {
                'S' => (Some(Screen::Standby), 0.5),
                'P' => (Some(Screen::Setup), 0.3),
                'Z' => (Some(Screen::Pause), 1.0),
                'F' => (None, 0.99),
                'L' => (Some(Screen::Live), 0.2),
                'G' => (Some(Screen::Live), 0.99),
                _ => (None, 0.2),
            };
            v.push(Sample { t: secs(t), screen, template: None, score: 0.9, still });
            t += 1.0 / FPS;
        }
    }
    v
}

fn cut_secs(c: &[Cut]) -> Vec<(f64, f64, CutReason)> {
    c.iter().map(|c| ((c.range.start.seconds() * 4.0).round() / 4.0, (c.range.end().seconds() * 4.0).round() / 4.0, c.reason)).collect()
}

fn run(runs: &[(f64, char)], p: &CutParams) -> Vec<(f64, f64, CutReason)> {
    let s = build(runs);
    cut_secs(&plan_cuts(&segments(&s, p), p))
}

#[test]
fn standby_then_setup_cut_until_ten_seconds_before_the_round() {
    let p = CutParams::default();
    // 60 s play, 120 s break screen, 30 s setup, 100 s play
    let c = run(&[(60.0, '.'), (120.0, 'S'), (30.0, 'P'), (100.0, '.')], &p);
    // spans are sample midpoints: the first standby sample at 60 s starts at 59.75
    assert_eq!(c, vec![(59.75, 199.75, CutReason::Waiting)]);
}

#[test]
fn unrecognised_time_between_break_and_setup_is_kept() {
    let p = CutParams::default();
    // a desk segment between the break screen and the setup is not waiting time
    let c = run(&[(10.0, '.'), (60.0, 'S'), (40.0, '.'), (30.0, 'P'), (20.0, '.')], &p);
    assert_eq!(c, vec![(9.75, 59.75, CutReason::Waiting), (109.75, 129.75, CutReason::Waiting)]);
}

#[test]
fn short_setup_inside_keep_before_is_not_cut() {
    let p = CutParams::default();
    // an 11 s setup leaves 1 s, under the 2 s minimum cut
    assert!(run(&[(10.0, '.'), (11.0, 'P'), (20.0, '.')], &p).is_empty());
}

#[test]
fn flicker_inside_a_break_screen_is_bridged() {
    let p = CutParams::default();
    let c = run(&[(5.0, '.'), (50.0, 'S'), (1.0, '.'), (50.0, 'S'), (20.0, '.')], &p);
    assert_eq!(c, vec![(4.75, 95.75, CutReason::Waiting)]);
}

#[test]
fn play_is_never_cut() {
    let p = CutParams::default();
    assert!(run(&[(600.0, '.')], &p).is_empty());
    // a long gap breaks a break screen in two: the play between is kept
    let c = run(&[(30.0, 'S'), (5.0, '.'), (30.0, 'S'), (5.0, '.')], &p);
    for (a, b, _) in &c {
        assert!(!(*a < 34.0 && *b > 30.5), "play between 30 and 35 s cut: {c:?}");
    }
}

#[test]
fn pause_notice_cut_whole_with_dissolve() {
    let p = CutParams::default();
    let s = build(&[(30.0, 'P'), (100.0, '.'), (45.0, 'Z'), (50.0, '.')]);
    let cuts = plan_cuts(&segments(&s, &p), &p);
    assert_eq!(cut_secs(&cuts), vec![(-0.25, 19.75, CutReason::Waiting), (129.75, 174.75, CutReason::Pause)]);
    assert!(cuts[1].dissolve && !cuts[0].dissolve);
}

#[test]
fn frozen_picture_is_a_pause_only_inside_a_round() {
    let p = CutParams::default();
    // frozen before any setup: not a pause (a static graphic before the show)
    assert!(run(&[(20.0, '.'), (30.0, 'F'), (20.0, '.')], &p).is_empty());
    // frozen after a setup ended: a pause; the notice in the middle joins it
    let c = run(&[(30.0, 'P'), (60.0, '.'), (10.0, 'F'), (20.0, 'Z'), (10.0, 'F'), (30.0, '.')], &p);
    assert_eq!(c, vec![(-0.25, 19.75, CutReason::Waiting), (89.75, 129.75, CutReason::Pause)]);
    // after the next break screen the round is over
    let c = run(&[(30.0, 'P'), (60.0, '.'), (40.0, 'S'), (5.0, '.'), (30.0, 'F'), (10.0, '.')], &p);
    assert_eq!(c.len(), 2, "{c:?}");
    assert!(c.iter().all(|x| x.2 == CutReason::Waiting));
}

#[test]
fn frozen_with_hud_is_a_pause_anywhere_and_without_it_never_when_hud_is_required() {
    let p = CutParams { frozen_needs_live: true, ..CutParams::default() };
    let c = run(&[(20.0, 'L'), (30.0, 'G'), (20.0, 'L')], &p);
    assert_eq!(c, vec![(19.75, 49.75, CutReason::Pause)]);
    // a frozen picture without the HUD inside a round: a static graphic, kept
    let c = run(&[(30.0, 'P'), (60.0, 'L'), (30.0, 'F'), (20.0, 'L')], &p);
    assert_eq!(c, vec![(-0.25, 19.75, CutReason::Waiting)]);
}

#[test]
fn short_freeze_is_kept() {
    let p = CutParams::default();
    assert_eq!(run(&[(30.0, 'P'), (60.0, '.'), (3.0, 'F'), (30.0, '.')], &p).len(), 1);
}

#[test]
fn keep_before_zero_cuts_the_whole_wait() {
    let p = CutParams { keep_before: Tick::ZERO, ..CutParams::default() };
    assert_eq!(run(&[(10.0, '.'), (30.0, 'P'), (10.0, '.')], &p), vec![(9.75, 39.75, CutReason::Waiting)]);
}

#[test]
fn hostile_samples_do_not_panic() {
    let p = CutParams { frozen_still: f32::NAN, min_cut: Tick(-5), keep_before: Tick(-100), ..CutParams::default() };
    let s = vec![Sample { t: Tick::MAX, screen: Some(Screen::Setup), template: None, score: 1.0, still: f32::NAN }];
    let _ = plan_cuts(&segments(&s, &p), &p);
    assert!(segments(&[], &p).is_empty());
    let mut a = Analyzer::new(TemplateSet::default());
    let f = Luma::from_gray(vec![0; 64], 8, 8).unwrap();
    assert!(a.push(secs(1.0), f.clone()).is_some());
    // out-of-order frames are ignored
    assert!(a.push(secs(0.5), f).is_none());
}
