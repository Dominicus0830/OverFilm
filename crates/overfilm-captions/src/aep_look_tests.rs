use super::*;

/// Real templates: `OVERFILM_AEP=<dir of .aep files> cargo test -p overfilm-captions -- --ignored real_templates --nocapture`.
#[test]
#[ignore]
fn real_templates() {
    let Ok(dir) = std::env::var("OVERFILM_AEP") else { return };
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for p in paths.into_iter().filter(|p| p.extension().is_some_and(|e| e == "aep")) {
        let t = template(&std::fs::read(&p).unwrap(), None);
        println!("== {}", p.display());
        let Some(t) = t else {
            println!("  none");
            continue;
        };
        println!("  took: {:?}\n  missed: {:?}\n  timing: {:?} scale {:.2} per_char {}", t.took, t.missed, t.look.timing, t.text_scale, t.per_character);
        if let Some(m) = &t.motion {
            println!("  in: {:?}", m.r#in.iter().map(|k| (k.t, k.scale, k.opacity, k.dx, k.dy)).collect::<Vec<_>>());
            println!("  out: {:?}", m.out.iter().map(|k| (k.t, k.scale, k.opacity, k.dx, k.dy)).collect::<Vec<_>>());
        }
        println!("  lines: {}", t.look.lines.len());
        for e in &t.look.effects {
            println!("  fx {}: {:?}", e.effect, e.params.iter().map(|p| (&p.id, &p.value, p.keys.len())).collect::<Vec<_>>());
        }
        println!("  gradient {:?} shadow {:?}", t.look.gradient, t.look.shadow);
        for s in &t.look.shapes {
            println!("  shape {:?}", (s.ellipse, s.size, s.at, s.fill, s.fit_text, s.front, s.motion.is_some()));
        }
        println!("  masks {}", t.look.masks.len());
    }
}

use crate::aep::tests::{effect, group, layer, ldta, named_prop, project, prop, square_mask, zoom_project};

#[test]
fn keyframed_motion_comes_over() {
    let t = template(&zoom_project(), None).unwrap();
    let m = t.motion.unwrap();
    assert_eq!(m.lengths(), (0.5, 0.5));
    let first = m.r#in.first().unwrap();
    assert!(first.scale < 1.0, "{first:?}");
    let last = m.out.last().unwrap();
    assert!((last.dy - 0.1).abs() < 0.01 && last.opacity < 1.0, "{last:?}");
    assert!(t.missed.is_empty(), "{:?}", t.missed);
}

/// A text parented to a null that rises into place, with a glow whose radius an expression reads
/// from a slider, a wiggle, a mask, a drop shadow and a second text layer.
fn rich_project() -> Vec<u8> {
    let null = layer(
        ldta(9, 0, 0, 0, "Mover"),
        &[group(
            "ADBE Transform Group",
            &[
                prop("ADBE Anchor Point", 3, true, &[0.0, 0.0, 0.0], &[], None),
                prop("ADBE Position", 3, true, &[960.0, 540.0, 0.0], &[(0, vec![960.0, 648.0], 1), (15, vec![960.0, 540.0], 1)], None),
            ],
        )],
    );
    let text = layer(
        ldta(1, 3, 9, 0, "Title"),
        &[
            group("ADBE Text Properties", &[]),
            square_mask([-100.0, -50.0, 100.0, 50.0]),
            group(
                "ADBE Effect Parade",
                &[
                    effect("ADBE Slider Control", "Glow Size", &[named_prop("ADBE Slider Control-0001", "Slider", &[1.3])]),
                    effect("ADBE Glo2", "Glow", &[prop("ADBE Glo2-0003", 1, false, &[10.0], &[], Some("effect(\"Glow Size\")(\"Slider\") * 20"))]),
                    effect("ADBE Drop Shadow", "Drop Shadow", &[named_prop("ADBE Drop Shadow-0004", "Distance", &[8.64])]),
                    effect("ADBE Nobody Plugin", "Mystery", &[]),
                ],
            ),
            group(
                "ADBE Transform Group",
                &[
                    prop("ADBE Anchor Point", 3, true, &[0.0, 0.0, 0.0], &[], None),
                    prop("ADBE Position", 3, true, &[0.0, 0.0, 0.0], &[], Some("wiggle(3, 12)")),
                ],
            ),
        ],
    );
    let second = layer(
        ldta(2, 3, 0, 0, "Sub"),
        &[
            group("ADBE Text Properties", &[]),
            group("ADBE Transform Group", &[prop("ADBE Opacity", 1, false, &[1.0], &[(0, vec![0.0], 1), (15, vec![1.0], 1)], None)]),
        ],
    );
    project(&[crate::aep::tests::comp(4, 30, 1920, 1080, 3, &[text, second, null])])
}

#[test]
fn effects_expressions_parents_and_masks_come_over() {
    let t = template(&rich_project(), Some(86.4)).unwrap();
    // the parent's rise is the text's motion in
    let m = t.motion.as_ref().expect("the parent's motion");
    let first = m.r#in.first().unwrap();
    assert!((first.dy - 0.1).abs() < 0.01, "{first:?}");
    // the expression-driven glow radius: 1.3 × 20 px = 26 px = 0.3 text heights
    let glow = t.look.effects.iter().find(|e| e.effect == "wonder_glow").expect("glow");
    let r = glow.params.iter().find(|p| p.id == "radius").unwrap();
    match r.value {
        FxValue::Len(x) => assert!((x - 26.0 / 86.4).abs() < 1e-3, "{x}"),
        ref v => panic!("{v:?}"),
    }
    assert!(t.look.effects.iter().any(|e| e.effect == "wiggle"), "{:?}", t.look.effects);
    let shadow = t.look.shadow.as_ref().expect("drop shadow");
    assert!((shadow.distance - 0.1).abs() < 1e-3, "{shadow:?}");
    assert_eq!(t.look.masks.len(), 1);
    let mask = &t.look.masks[0];
    assert_eq!((mask.mode, mask.inverted), (2, true));
    assert_eq!(mask.path.first().map(|k| (k.closed, k.vertices.len())), Some((true, 4)));
    assert_eq!(t.look.lines.len(), 1, "the second text moves line two");
    assert!(t.missed.iter().any(|s| s.contains("Mystery") || s.contains("Nobody")), "{:?}", t.missed);
    // labels are not repeated
    let mut took = t.took.clone();
    took.dedup();
    assert_eq!(took, t.took);
}

#[test]
fn hostile_templates_never_panic() {
    let good = rich_project();
    for n in (0..good.len()).step_by(7) {
        let _ = template(&good[..n], Some(40.0));
    }
    for i in (8..good.len()).step_by(5) {
        for x in [0u8, 0x7f, 0xff] {
            let mut b = good.clone();
            if let Some(v) = b.get_mut(i) {
                *v = x;
            }
            let _ = template(&b, None);
            let _ = template(&b, Some(f64::NAN));
        }
    }
}

#[test]
fn an_opaque_box_over_the_text_is_left_out() {
    let card = ShapeLook { size: [12.0, 7.0], at: [5.0, 2.7], fill: Some([255, 255, 255, 255]), front: true, ..ShapeLook::default() };
    assert!(hides_text(&card));
    assert!(!hides_text(&ShapeLook { front: false, ..card.clone() }));
    assert!(!hides_text(&ShapeLook { fill: Some([255, 255, 255, 100]), ..card.clone() }));
    assert!(hides_text(&ShapeLook { size: [7.6, 4.3], at: [2.4, 1.67], ..card.clone() }));
    assert!(!hides_text(&ShapeLook { size: [21.0, 1.8], at: [0.4, 2.0], ..card.clone() }), "a bar beside the text");
    assert!(!hides_text(&ShapeLook { size: [7.0, 0.5], at: [0.0, 0.0], ..card }), "an underline");
}
