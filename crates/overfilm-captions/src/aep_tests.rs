use super::*;

fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = tag.to_vec();
    v.extend((body.len() as u32).to_be_bytes());
    v.extend(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
    let mut body = kind.to_vec();
    for p in parts {
        body.extend(p);
    }
    chunk(b"LIST", &body)
}

fn tdmn(name: &str) -> Vec<u8> {
    let mut b = name.as_bytes().to_vec();
    b.resize(40, 0);
    chunk(b"tdmn", &b)
}

/// Keyframes: (time units, value(s), interpolation out). `spatial` puts the value at the 7th f64.
fn keys(keys: &[(u32, [f64; 2], u8)], spatial: bool, dims: usize) -> Vec<u8> {
    let size = if spatial || dims > 1 { 128 } else { 48 };
    let mut lhd3 = vec![0u8; 52];
    lhd3[8..12].copy_from_slice(&(keys.len() as u32).to_be_bytes());
    lhd3[16..20].copy_from_slice(&(size as u32).to_be_bytes());
    let mut ldat = Vec::new();
    for (t, v, interp) in keys {
        let mut r = vec![0u8; size];
        r[0..4].copy_from_slice(&t.to_be_bytes());
        r[4] = *interp;
        r[5] = *interp;
        let at = if spatial { 6 } else { 0 };
        for (i, x) in v.iter().take(dims).enumerate() {
            r[8 + 8 * (at + i)..16 + 8 * (at + i)].copy_from_slice(&x.to_be_bytes());
        }
        ldat.extend(r);
    }
    list(b"list", &[chunk(b"lhd3", &lhd3), chunk(b"ldat", &ldat)])
}

fn cdta(rate: u32) -> Vec<u8> {
    let mut b = vec![0u8; 204];
    b[8..12].copy_from_slice(&rate.to_be_bytes());
    chunk(b"cdta", &b)
}

/// A composition (time base 30 000) with a text layer that pops in (scale 0 → 100 % in 0.5 s,
/// eased), rests until 2.5 s, then slides down 108 px while fading out by 3 s.
pub(crate) fn zoom_project() -> Vec<u8> {
    let layer = list(
        b"Layr",
        &[
            list(b"tdgp", &[tdmn("ADBE Text Properties")]),
            list(
                b"tdgp",
                &[
                    tdmn("ADBE Transform Group"),
                    tdmn("ADBE Scale"),
                    list(b"tdbs", &[keys(&[(0, [0.0, 0.0], 2), (15000, [1.0, 1.0], 1), (75000, [1.0, 1.0], 1), (90000, [1.0, 1.0], 1)], false, 2)]),
                    tdmn("ADBE Position"),
                    list(b"tdbs", &[keys(&[(0, [960.0, 900.0], 1), (75000, [960.0, 900.0], 1), (90000, [960.0, 1008.0], 1)], true, 2)]),
                    tdmn("ADBE Opacity"),
                    list(b"tdbs", &[keys(&[(0, [1.0, 0.0], 1), (75000, [1.0, 0.0], 1), (90000, [0.0, 0.0], 1)], false, 1)]),
                ],
            ),
        ],
    );
    let item = list(b"Item", &[cdta(30000), layer]);
    let mut body = b"Egg!".to_vec();
    body.extend(item);
    let mut v = b"RIFX".to_vec();
    v.extend((body.len() as u32).to_be_bytes());
    v.extend(body);
    v
}

#[test]
fn a_text_layers_keyframes_become_motion_in_and_out() {
    let m = motion(&zoom_project(), 1080.0).unwrap();
    assert!(!m.per_character);
    let c = m.custom.unwrap();
    // in: 0 → 0.5 s, from 0 % to 100 %, eased first
    assert_eq!(c.r#in.len(), 2, "{c:?}");
    assert_eq!(c.r#in[0].scale, 0.0);
    assert!(c.r#in[0].ease);
    assert!((c.r#in[1].t - 0.5).abs() < 1e-6);
    assert_eq!(c.r#in[1].scale, 100.0);
    // out: from rest at 2.5 s to 3 s: down 10 % of the frame and gone
    assert_eq!(c.out.len(), 2);
    assert_eq!(c.out[0].t, 0.0);
    assert_eq!(c.out[0].opacity, 100.0);
    assert!((c.out[1].t - 0.5).abs() < 1e-6);
    assert!((c.out[1].dy - 0.1).abs() < 1e-6);
    assert_eq!(c.out[1].opacity, 0.0);
    assert_eq!(c.lengths(), (0.5, 0.5));
}

#[test]
fn a_per_character_animator_is_noted() {
    let layer = list(
        b"Layr",
        &[tdmn("ADBE Text Properties"), tdmn("ADBE Text Percent Offset"), list(b"tdbs", &[keys(&[(0, [0.0, 0.0], 2), (30000, [1.0, 0.0], 2)], false, 1)])],
    );
    let mut body = b"Egg!".to_vec();
    body.extend(list(b"Item", &[cdta(30000), layer]));
    let mut v = b"RIFX".to_vec();
    v.extend((body.len() as u32).to_be_bytes());
    v.extend(body);
    let m = motion(&v, 1080.0).unwrap();
    assert!(m.per_character);
    assert_eq!(m.custom, None);
}

#[test]
fn hostile_projects_never_panic() {
    assert_eq!(motion(b"", 1080.0), None);
    assert_eq!(motion(b"RIFX", 1080.0), None);
    assert_eq!(motion(b"RIFX\xff\xff\xff\xffEgg!LIST\xff\xff\xff\xffLayr", f64::NAN), None);
    let good = zoom_project();
    for n in 0..good.len() {
        let _ = motion(&good[..n], 1080.0);
        for x in [0u8, 0x7f, 0xff] {
            let mut bad = good.clone();
            bad[n] = x;
            let _ = motion(&bad, 0.0);
        }
    }
}

/// Real templates: `OVERFILM_AEP=<dir of .aep files> cargo test -p overfilm-captions -- --ignored real_aep --nocapture`.
#[test]
#[ignore]
fn real_aep() {
    let Ok(dir) = std::env::var("OVERFILM_AEP") else { return };
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for p in paths.into_iter().filter(|p| p.extension().is_some_and(|e| e == "aep")) {
        let m = motion(&std::fs::read(&p).unwrap(), 1080.0);
        println!("{}: {m:?}", p.display());
    }
}
