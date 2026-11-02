use super::*;

pub(crate) fn chunk(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = tag.to_vec();
    v.extend((body.len() as u32).to_be_bytes());
    v.extend(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

pub(crate) fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
    let mut body = kind.to_vec();
    for p in parts {
        body.extend(p);
    }
    chunk(b"LIST", &body)
}

pub(crate) fn tdmn(name: &str) -> Vec<u8> {
    let mut b = name.as_bytes().to_vec();
    b.resize(40, 0);
    chunk(b"tdmn", &b)
}

pub(crate) fn tdsn(name: &str) -> Vec<u8> {
    chunk(b"tdsn", &chunk(b"Utf8", name.as_bytes()))
}

/// A property's `tdb4`: dimensions, spatial, colour.
pub(crate) fn tdb4(dims: u16, spatial: bool, color: bool) -> Vec<u8> {
    let mut b = vec![0u8; 124];
    b[0..2].copy_from_slice(&0xdb99u16.to_be_bytes());
    b[2..4].copy_from_slice(&dims.to_be_bytes());
    if spatial {
        b[5] = 0x08;
    }
    if color {
        b[59] = 0x01;
    }
    chunk(b"tdb4", &b)
}

pub(crate) fn cdat(v: &[f64]) -> Vec<u8> {
    chunk(b"cdat", &v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>())
}

/// A keyframe `LIST list`: (time units, value(s), out interpolation). Spatial values sit after
/// five `f64`s, colours after six, others first.
pub(crate) fn keys(keys: &[(i32, Vec<f64>, u8)], dims: usize, spatial: bool, color: bool) -> Vec<u8> {
    let size = if color {
        152
    } else if spatial {
        8 + 8 * (6 + 3 * dims)
    } else {
        8 + 8 * 5 * dims
    };
    let mut lhd3 = vec![0u8; 52];
    lhd3[8..12].copy_from_slice(&(keys.len() as u32).to_be_bytes());
    lhd3[16..20].copy_from_slice(&(size as u32).to_be_bytes());
    let mut ldat = Vec::new();
    for (t, v, interp) in keys {
        let mut r = vec![0u8; size];
        r[0..4].copy_from_slice(&t.to_be_bytes());
        r[4] = *interp;
        r[5] = *interp;
        let at = if spatial || color { 6 } else { 0 };
        for (i, x) in v.iter().enumerate() {
            r[8 + 8 * (at + i)..16 + 8 * (at + i)].copy_from_slice(&x.to_be_bytes());
        }
        if *interp == 2 && !spatial && !color {
            // eased: 33 % influence, zero speed, per dimension
            for i in 0..dims {
                for part in [2, 4] {
                    let o = 8 + 8 * (part * dims + i);
                    r[o..o + 8].copy_from_slice(&(1.0f64 / 3.0).to_be_bytes());
                }
            }
        }
        ldat.extend(r);
    }
    list(b"list", &[chunk(b"lhd3", &lhd3), chunk(b"ldat", &ldat)])
}

/// A property: its match name and `LIST tdbs`.
pub(crate) fn prop(name: &str, dims: u16, spatial: bool, still: &[f64], k: &[(i32, Vec<f64>, u8)], expr: Option<&str>) -> Vec<u8> {
    let mut parts = vec![tdb4(dims, spatial, false), cdat(still)];
    if !k.is_empty() {
        parts.push(keys(k, usize::from(dims), spatial, false));
    }
    if let Some(e) = expr {
        parts.push(chunk(b"Utf8", e.as_bytes()));
    }
    [tdmn(name), list(b"tdbs", &parts)].concat()
}

pub(crate) fn named_prop(name: &str, display: &str, still: &[f64]) -> Vec<u8> {
    [tdmn(name), list(b"tdbs", &[tdsn(display), tdb4(still.len() as u16, false, false), cdat(still)])].concat()
}

pub(crate) fn idta(kind: u16, id: u32) -> Vec<u8> {
    let mut b = vec![0u8; 80];
    b[0..2].copy_from_slice(&kind.to_be_bytes());
    b[16..20].copy_from_slice(&id.to_be_bytes());
    chunk(b"idta", &b)
}

pub(crate) fn cdta(rate: u32, w: u16, h: u16, secs: u32) -> Vec<u8> {
    let mut b = vec![0u8; 204];
    b[8..12].copy_from_slice(&rate.to_be_bytes());
    b[44..48].copy_from_slice(&(secs * rate).to_be_bytes());
    b[48..52].copy_from_slice(&rate.to_be_bytes());
    b[140..142].copy_from_slice(&w.to_be_bytes());
    b[142..144].copy_from_slice(&h.to_be_bytes());
    chunk(b"cdta", &b)
}

/// `ldta`: id, type (0 footage / precomp, 3 text, 4 shape), parent, source, name.
pub(crate) fn ldta(id: u32, kind: u8, parent: u32, source: u32, name: &str) -> Vec<u8> {
    let mut b = vec![0u8; 164];
    b[0..4].copy_from_slice(&id.to_be_bytes());
    for o in [16, 24, 32] {
        b[o..o + 4].copy_from_slice(&1u32.to_be_bytes());
    }
    b[40..44].copy_from_slice(&source.to_be_bytes());
    let n = name.as_bytes();
    b[64..64 + n.len().min(31)].copy_from_slice(&n[..n.len().min(31)]);
    b[39] = 0x07;
    b[131] = kind;
    b[132..136].copy_from_slice(&parent.to_be_bytes());
    chunk(b"ldta", &b)
}

/// A layer: `ldta` then its groups.
pub(crate) fn layer(ldta: Vec<u8>, groups: &[Vec<u8>]) -> Vec<u8> {
    let mut parts = vec![ldta];
    parts.push(list(b"tdgp", groups));
    list(b"Layr", &parts)
}

/// An effect as After Effects writes it: its match name, then `LIST sspc` with the parameter
/// definitions and a group of values named `display`.
pub(crate) fn effect(name: &str, display: &str, params: &[Vec<u8>]) -> Vec<u8> {
    let mut values = vec![tdsn(display)];
    values.extend(params.iter().cloned());
    [tdmn(name), list(b"sspc", &[chunk(b"fnam", &chunk(b"Utf8", name.as_bytes())), list(b"parT", &[tdmn(&format!("{name}-0001"))]), list(b"tdgp", &values)])]
        .concat()
}

pub(crate) fn group(name: &str, parts: &[Vec<u8>]) -> Vec<u8> {
    [tdmn(name), list(b"tdgp", parts)].concat()
}

pub(crate) fn comp(id: u32, rate: u32, w: u16, h: u16, secs: u32, layers: &[Vec<u8>]) -> Vec<u8> {
    let mut parts = vec![idta(4, id), cdta(rate, w, h, secs)];
    parts.extend(layers.iter().cloned());
    list(b"Item", &parts)
}

pub(crate) fn project(items: &[Vec<u8>]) -> Vec<u8> {
    let mut body = b"Egg!".to_vec();
    for i in items {
        body.extend(i);
    }
    let mut v = b"RIFX".to_vec();
    v.extend((body.len() as u32).to_be_bytes());
    v.extend(body);
    v
}

/// A 1920 × 1080 composition (time base 30 000) with a text layer that pops in (scale 0 →
/// 100 % in 0.5 s, linear), rests until 2.5 s, then slides down 108 px while fading out by 3 s.
pub(crate) fn zoom_project() -> Vec<u8> {
    let text = layer(
        ldta(1, 3, 0, 0, "Title"),
        &[
            group("ADBE Text Properties", &[]),
            group(
                "ADBE Transform Group",
                &[
                    prop("ADBE Anchor Point", 3, true, &[0.0, -40.0, 0.0], &[], None),
                    prop(
                        "ADBE Position",
                        3,
                        true,
                        &[960.0, 900.0, 0.0],
                        &[(0, vec![960.0, 900.0], 1), (75000, vec![960.0, 900.0], 1), (90000, vec![960.0, 1008.0], 1)],
                        None,
                    ),
                    prop("ADBE Scale", 3, false, &[1.0, 1.0, 1.0], &[(0, vec![0.0, 0.0, 0.0], 1), (15000, vec![1.0, 1.0, 1.0], 1)], None),
                    prop("ADBE Opacity", 1, false, &[1.0], &[(0, vec![1.0], 1), (75000, vec![1.0], 1), (90000, vec![0.0], 1)], None),
                ],
            ),
        ],
    );
    project(&[comp(7, 30000, 1920, 1080, 3, &[text])])
}

/// A mask parade with one closed square mask (bounding box `bbox`), Subtract, inverted, feather 12.
pub(crate) fn square_mask(bbox: [f32; 4]) -> Vec<u8> {
    let mut mask_shape = Vec::new();
    let mut shph = vec![0u8; 24];
    shph[3] = 1;
    for (i, v) in bbox.iter().enumerate() {
        shph[4 + 4 * i..8 + 4 * i].copy_from_slice(&v.to_be_bytes());
    }
    let pts: Vec<[f32; 2]> =
        vec![[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [1.0, 1.0], [1.0, 1.0], [0.0, 1.0], [0.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
    let mut lhd3 = vec![0u8; 52];
    lhd3[8..12].copy_from_slice(&(pts.len() as u32).to_be_bytes());
    lhd3[16..20].copy_from_slice(&8u32.to_be_bytes());
    let ldat: Vec<u8> = pts.iter().flat_map(|p| [p[0].to_be_bytes(), p[1].to_be_bytes()].concat()).collect();
    mask_shape.extend(tdmn("ADBE Mask Shape"));
    mask_shape.extend(list(
        b"om-s",
        &[
            list(b"tdbs", &[tdb4(1, false, false)]),
            list(b"omks", &[list(b"shap", &[chunk(b"shph", &shph), list(b"list", &[chunk(b"lhd3", &lhd3), chunk(b"ldat", &ldat)])])]),
        ],
    ));
    let mut mkif = vec![0u8; 48];
    mkif[0] = 1;
    mkif[6..8].copy_from_slice(&2u16.to_be_bytes());
    [
        tdmn("ADBE Mask Parade"),
        list(
            b"tdgp",
            &[tdmn("ADBE Mask Atom"), list(b"tdgp", &[mask_shape, named_prop("ADBE Mask Feather", "Feather", &[12.0, 12.0])]), chunk(b"mkif", &mkif)],
        ),
    ]
    .concat()
}

#[test]
fn layers_props_effects_and_masks_are_read() {
    // a closed square path 0.25..0.75 of the layer
    let text = layer(
        ldta(5, 3, 9, 0, "Main"),
        &[
            square_mask([0.25, 0.25, 0.75, 0.75]),
            group(
                "ADBE Effect Parade",
                &[
                    effect("ADBE Glo2", "My Glow", &[named_prop("ADBE Glo2-0003", "Glow Radius", &[53.0])]),
                    effect("ADBE Slider Control", "Dur", &[named_prop("ADBE Slider Control-0001", "Slider", &[1.3])]),
                ],
            ),
            group("ADBE Transform Group", &[prop("ADBE Opacity", 1, false, &[1.0], &[], Some("thisComp.layer(\"Null\").transform.opacity"))]),
        ],
    );
    let null = layer(ldta(9, 0, 0, 0, "Null"), &[]);
    let p = read(&project(&[comp(3, 30, 1280, 720, 4, &[text, null])])).unwrap();
    let c = &p.comps[0];
    assert_eq!((c.id, c.w, c.h, c.duration, c.rate), (3, 1280.0, 720.0, 4.0, 30.0));
    let l = &c.layers[0];
    assert_eq!((l.id, l.kind, l.parent, l.name.as_str()), (5, LayerKind::Text, 9, "Main"));
    assert_eq!(l.effects.len(), 2);
    assert_eq!(l.effects[0].display, "My Glow");
    assert_eq!(l.effects[0].param(3).unwrap().value, vec![53.0]);
    assert_eq!(l.effects[1].param(1).unwrap().display, "Slider");
    assert_eq!(l.prop("ADBE Opacity").unwrap().expr.as_deref(), Some("thisComp.layer(\"Null\").transform.opacity"));
    let m = &l.masks[0];
    assert_eq!((m.mode, m.inverted), (2, true));
    assert_eq!(m.feather.as_ref().unwrap().value, vec![12.0, 12.0]);
    let (_, _, path) = &m.shapes[0];
    assert!(path.closed);
    assert_eq!(path.pts[0], [0.25, 0.25]);
    assert_eq!(path.pts[3], [0.75, 0.25]);
}

#[test]
fn a_per_character_animator_is_noted() {
    let text = layer(
        ldta(1, 3, 0, 0, "T"),
        &[group(
            "ADBE Text Properties",
            &[group("ADBE Text Animators", &[prop("ADBE Text Percent Offset", 1, false, &[0.0], &[(0, vec![0.0], 2), (30000, vec![100.0], 2)], None)])],
        )],
    );
    let p = read(&project(&[comp(1, 30000, 1920, 1080, 2, &[text])])).unwrap();
    assert!(p.comps[0].layers[0].per_character);
}

#[test]
fn hostile_projects_never_panic() {
    assert_eq!(read(b""), None);
    assert_eq!(read(b"RIFX"), None);
    assert_eq!(read(b"RIFX\xff\xff\xff\xffEgg!LIST\xff\xff\xff\xffLayr"), None);
    let good = zoom_project();
    for n in 0..good.len() {
        let _ = crate::aep_look::template(&good[..n], None);
        for x in [0u8, 0x7f, 0xff] {
            let mut bad = good.clone();
            bad[n] = x;
            let _ = crate::aep_look::template(&bad, Some(f64::NAN));
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
        let m = read(&std::fs::read(&p).unwrap());
        println!("{}: {} comps", p.display(), m.map_or(0, |m| m.comps.len()));
    }
}
