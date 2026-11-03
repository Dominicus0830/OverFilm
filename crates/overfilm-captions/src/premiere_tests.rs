use super::*;

/// A value in a hand-built FlatBuffers table (the layout `flat_style` reads).
enum V {
    U8(u8),
    F32(f32),
    Table(Vec<(usize, V)>),
    Str(String),
    Tables(Vec<Vec<(usize, V)>>),
    Strs(Vec<String>),
}

#[derive(Default)]
struct B(Vec<u8>);

impl B {
    fn align(&mut self, n: usize) {
        while !self.0.len().is_multiple_of(n) {
            self.0.push(0);
        }
    }
    fn patch(&mut self, slot: usize, target: usize) {
        self.0[slot..slot + 4].copy_from_slice(&((target - slot) as u32).to_le_bytes());
    }
    /// vtable, then the table (one 4-byte slot per field), then its children; returns the table.
    fn table(&mut self, fields: &[(usize, V)]) -> usize {
        self.align(2);
        let max = fields.iter().map(|(i, _)| *i + 1).max().unwrap_or(0);
        let vt = self.0.len();
        let vt_len = 4 + 2 * max;
        self.0.resize(vt + vt_len, 0);
        // pad so the table starts 4-aligned
        self.align(4);
        let pos = self.0.len();
        let size = 4 + 4 * fields.len();
        self.0.resize(pos + size, 0);
        self.0[pos..pos + 4].copy_from_slice(&((pos - vt) as i32).to_le_bytes());
        self.0[vt..vt + 2].copy_from_slice(&(vt_len as u16).to_le_bytes());
        self.0[vt + 2..vt + 4].copy_from_slice(&(size as u16).to_le_bytes());
        let mut later = Vec::new();
        for (k, (i, v)) in fields.iter().enumerate() {
            let slot = pos + 4 + 4 * k;
            self.0[vt + 4 + 2 * i..vt + 6 + 2 * i].copy_from_slice(&((slot - pos) as u16).to_le_bytes());
            match v {
                V::U8(x) => self.0[slot] = *x,
                V::F32(x) => self.0[slot..slot + 4].copy_from_slice(&x.to_le_bytes()),
                _ => later.push((slot, v)),
            }
        }
        for (slot, v) in later {
            let target = self.child(v);
            self.patch(slot, target);
        }
        pos
    }
    fn string(&mut self, s: &str) -> usize {
        self.align(4);
        let p = self.0.len();
        self.0.extend((s.len() as u32).to_le_bytes());
        self.0.extend(s.as_bytes());
        self.0.push(0);
        p
    }
    fn child(&mut self, v: &V) -> usize {
        match v {
            V::Table(f) => self.table(f),
            V::Str(s) => self.string(s),
            V::Tables(ts) => {
                self.align(4);
                let p = self.0.len();
                self.0.extend((ts.len() as u32).to_le_bytes());
                self.0.resize(p + 4 + 4 * ts.len(), 0);
                for (k, t) in ts.iter().enumerate() {
                    let at = self.table(t);
                    self.patch(p + 4 + 4 * k, at);
                }
                p
            }
            V::Strs(ss) => {
                self.align(4);
                let p = self.0.len();
                self.0.extend((ss.len() as u32).to_le_bytes());
                self.0.resize(p + 4 + 4 * ss.len(), 0);
                for (k, s) in ss.iter().enumerate() {
                    let at = self.string(s);
                    self.patch(p + 4 + 4 * k, at);
                }
                p
            }
            V::U8(_) | V::F32(_) => unreachable!(),
        }
    }
}

fn colour(r: Option<u8>, g: u8, b: u8) -> V {
    let mut f = Vec::new();
    if let Some(r) = r {
        f.push((0, V::U8(r)));
    }
    f.push((1, V::U8(g)));
    f.push((2, V::U8(b)));
    V::Table(f)
}

/// A `Source Text` value as current Premiere writes it: length, marker, FlatBuffers.
fn blob(text_doc: Vec<(usize, V)>) -> Vec<u8> {
    let mut b = B::default();
    b.0.resize(4, 0);
    let root = b.table(&[(0, V::Table(text_doc))]);
    b.patch(0, root);
    let mut out = (b.0.len() as u64).to_le_bytes().to_vec();
    out.extend([0x44, 0x33, 0x22, 0x11]);
    out.extend(b.0);
    out
}

fn b64(d: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in d.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// A text style shaped like the ones Premiere exports: outlined yellow text on a 60 % black box.
fn yellow_boxed() -> Vec<u8> {
    let style = vec![(1, V::F32(72.0)), (2, colour(None, 220, 0)), (4, colour(Some(0), 0, 0)), (5, V::U8(1)), (6, V::F32(8.0))];
    let run = vec![(0, V::Str("Aa".into())), (1, V::Table(style))];
    blob(vec![
        (0, V::Tables(vec![run])),
        (1, V::Strs(vec!["NanumSquare-ExtraBold".into()])),
        (11, V::U8(1)),
        (17, colour(Some(0), 0, 0)),
        (18, V::U8(1)),
        (19, V::F32(60.0)),
        (20, V::F32(18.0)),
        (34, V::F32(9.0)),
    ])
}

fn prtextstyle(name: &str, blob: &[u8]) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<PremiereData Version=\"3\">\n\t<StyleProjectItem ObjectUID=\"x\" Version=\"1\">\n\t\t<ProjectItem Version=\"1\">\n\t\t\t<Name>{name}</Name>\n\t\t</ProjectItem>\n\t</StyleProjectItem>\n\t<ArbVideoComponentParam ObjectID=\"40\" Version=\"2\">\n\t\t<Name>Source Text</Name>\n\t\t<StartKeyframeValue Encoding=\"base64\" BinaryHash=\"0\">{}</StartKeyframeValue>\n\t</ArbVideoComponentParam>\n</PremiereData>\n",
        b64(blob)
    )
}

#[test]
fn a_prtextstyle_becomes_a_preset() {
    let xml = prtextstyle("내 노랑 &amp; 박스", &yellow_boxed());
    let r = import(xml.as_bytes(), "file").unwrap();
    assert_eq!(r.presets.len(), 1);
    let p = &r.presets[0];
    assert_eq!(p.name, "내 노랑 & 박스");
    assert_eq!(r.fonts, vec!["NanumSquare-ExtraBold".to_string()]);
    assert_eq!(p.font_style, "ExtraBold");
    assert_eq!(p.size, 72.0);
    // red left out = 255 (the format's default)
    assert_eq!(p.fill, [255, 220, 0, 255]);
    assert_eq!(p.stroke, Some(Stroke { color: [0, 0, 0, 255], width: 8.0 }));
    assert_eq!(p.boxed, Some(BoxStyle { color: [0, 0, 0, 153], padding: 18.0, radius: 9.0 }));
    assert!(p.shadow);
}

#[test]
fn outline_and_box_off_stay_off() {
    let style = vec![(1, V::F32(50.0)), (4, colour(Some(10), 10, 10)), (6, V::F32(4.0))];
    let run = vec![(0, V::Str("Aa".into())), (1, V::Table(style))];
    let b = blob(vec![(0, V::Tables(vec![run])), (1, V::Strs(vec!["Roboto-Regular".into()])), (17, colour(Some(255), 255, 255)), (20, V::F32(20.0))]);
    let r = import(prtextstyle("깔끔", &b).as_bytes(), "x").unwrap();
    let p = &r.presets[0];
    assert_eq!(p.fill, [255, 255, 255, 255]);
    assert_eq!(p.stroke, None);
    assert_eq!(p.boxed, None);
    assert!(!p.shadow);
}

#[test]
fn a_premiere_project_or_xml_gives_each_different_look_once() {
    let a = yellow_boxed();
    let style = vec![(1, V::F32(40.0))];
    let other = blob(vec![(0, V::Tables(vec![vec![(0, V::Str("x".into())), (1, V::Table(style))]])), (1, V::Strs(vec!["Inter-Bold".into()]))]);
    let xml = format!(
        "<xmeml><sequence><effect><name>Source Text</name><value>{}</value></effect><effect><value>{}</value></effect><effect><value>{}</value></effect><x>QUJD</x></sequence></xmeml>",
        b64(&a),
        b64(&a),
        b64(&other)
    );
    let r = import(xml.as_bytes(), "편집").unwrap();
    assert_eq!(r.presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["편집", "편집 2"]);
    assert_eq!(r.fonts, ["NanumSquare-ExtraBold", "Inter-Bold"]);
    // gzip (.prproj)
    let gz = gzip(xml.as_bytes());
    assert_eq!(import(&gz, "편집").unwrap().presets.len(), 2);
}

fn gzip(d: &[u8]) -> Vec<u8> {
    let mut v = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    v.extend(miniz_oxide::deflate::compress_to_vec(d, 6));
    v.extend([0; 8]);
    v
}

/// A ZIP with the given entries (deflated when `deflate`).
fn zip(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let body = if deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
        let local = out.len() as u32;
        let method: u16 = if deflate { 8 } else { 0 };
        out.extend(b"PK\x03\x04");
        out.extend([20, 0, 0, 0]);
        out.extend(method.to_le_bytes());
        out.extend([0; 8]); // time, date, crc
        out.extend((body.len() as u32).to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend([0, 0]);
        out.extend(name.as_bytes());
        out.extend(&body);
        central.extend(b"PK\x01\x02");
        central.extend([20, 0, 20, 0, 0, 0]);
        central.extend(method.to_le_bytes());
        central.extend([0; 8]);
        central.extend((body.len() as u32).to_le_bytes());
        central.extend((data.len() as u32).to_le_bytes());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend([0; 12]);
        central.extend(local.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let at = out.len() as u32;
    let n = entries.len() as u16;
    out.extend(&central);
    out.extend(b"PK\x05\x06");
    out.extend([0, 0, 0, 0]);
    out.extend(n.to_le_bytes());
    out.extend(n.to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(at.to_le_bytes());
    out.extend([0, 0]);
    out
}

const AE_DEFINITION: &str = r#"{
  "authorApp": "aefx",
  "capsuleName": "Pop Subtitle",
  "capsuleNameLocalized": {"strDB": [{"localeString": "ko_KR", "str": "팝 자막"}]},
  "clientControls": [
    {"type": 6, "uiName": {"strDB": [{"str": "Text"}]}, "fonteditinfo": {"fontEditValue": "Pretendard-Black", "fontSizeEditValue": 140}, "value": {"strDB": [{"str": "자막"}]}},
    {"type": 4, "uiName": {"strDB": [{"str": "Text Color"}]}, "value": [1, 0.5, 0, 1]},
    {"type": 4, "uiName": {"strDB": [{"str": "Stroke Color"}]}, "value": [0, 0, 0, 1]},
    {"type": 2, "uiName": {"strDB": [{"str": "Stroke Width"}]}, "value": 10},
    {"type": 4, "uiName": {"strDB": [{"str": "Background Color"}]}, "value": [0.1, 0.1, 0.1, 1]},
    {"type": 2, "uiName": {"strDB": [{"str": "BG Opacity"}]}, "value": 50},
    {"type": 4, "uiName": {"strDB": [{"str": "Gradient Ramp Start Color"}]}, "value": [0, 1, 0, 1]}
  ],
  "sourceInfoLocalized": {"en_US": {"framesize": {"size": {"x": 3840, "y": 2160}}}}
}"#;

#[test]
fn an_after_effects_mogrt_gives_font_colours_and_motion() {
    for deflate in [false, true] {
        let z = zip(&[("thumb.png", b"\x89PNG"), ("definition.json", AE_DEFINITION.as_bytes()), ("project.aegraphic", b"PK\x03\x04junk")], deflate);
        let r = import(&z, "file").unwrap();
        let p = &r.presets[0];
        assert_eq!(p.name, "팝 자막");
        assert_eq!(r.fonts, ["Pretendard-Black"]);
        assert_eq!(p.font_style, "Black");
        // 140 at 2160 lines = 70 at 1080
        assert_eq!(p.size, 70.0);
        assert_eq!(p.fill, [255, 128, 0, 255]);
        assert_eq!(p.stroke, Some(Stroke { color: [0, 0, 0, 255], width: 5.0 }));
        assert_eq!(p.boxed.as_ref().map(|b| b.color), Some([26, 26, 26, 128]));
        assert_eq!(p.motion_in, Motion::Pop);
        assert_eq!(p.motion_out, Motion::Fade);
        assert!(!r.notes.is_empty());
    }
}

#[test]
fn an_after_effects_mogrt_brings_its_keyframed_motion() {
    let aegraphic = zip(&[("anim.aep", &crate::aep::tests::zoom_project()), ("animReport.txt", b"report")], true);
    let def = AE_DEFINITION.replace("Pop Subtitle", "Title").replace("팝 자막", "제목");
    let z = zip(&[("definition.json", def.as_bytes()), ("project.aegraphic", &aegraphic)], true);
    let r = import(&z, "file").unwrap();
    let p = &r.presets[0];
    let c = p.custom_motion.as_ref().expect("the template's own motion");
    assert_eq!(c.lengths(), (0.5, 0.5));
    assert_eq!(p.motion_in, Motion::None);
    assert!(r.notes.iter().any(|n| n.contains("움직임")), "{:?}", r.notes);
    // the caption it makes moves like the template
    let e = crate::styles::caption_layer(p, "제목", None, filmcraft_time::Tick::from_seconds_f64(3.0), crate::styles::Frame { width: 1920, height: 1080 });
    let sc = &e.params["scale"].keyframes;
    assert_eq!(sc.first().map(|k| k.value.clone()), Some(filmcraft_project::ParamValue::Float(0.0)));
    assert!(e.params["opacity"].is_animated());
    assert!(e.params["position"].is_animated());
}

#[test]
fn a_premiere_mogrt_uses_its_text_style_and_name() {
    let xml = prtextstyle("ignored", &yellow_boxed());
    let inner = zip(&[("graphic.xml", xml.as_bytes())], true);
    let def = r#"{"capsuleName": "Slide Up Lower Third", "clientControls": []}"#;
    let z = zip(&[("definition.json", def.as_bytes()), ("project.prgraphic", &inner)], false);
    let r = import(&z, "file").unwrap();
    assert_eq!(r.presets.len(), 1);
    let p = &r.presets[0];
    assert_eq!(p.fill, [255, 220, 0, 255]);
    assert_eq!(p.motion_in, Motion::SlideUp);
}

#[test]
fn postscript_names_split_into_family_and_style() {
    assert_eq!(split_postscript("Roboto-BoldItalic"), ("Roboto".into(), "BoldItalic".into()));
    assert_eq!(split_postscript("KomikaAxis"), ("KomikaAxis".into(), "Regular".into()));
    assert_eq!(split_postscript("-x"), ("-x".into(), "Regular".into()));
}

#[test]
fn hostile_files_are_refused_without_panicking() {
    assert!(import(b"", "x").is_err());
    assert!(import(b"PK\x03\x04", "x").is_err());
    assert!(import(b"PK\x05\x06\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff", "x").is_err());
    assert!(import(&[0x1f, 0x8b, 8, 0xff], "x").is_err());
    assert!(import(b"<a>AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA</a>", "x").is_err());
    // a valid header with garbage offsets: every truncation and every corrupted byte
    let good = yellow_boxed();
    for n in 0..good.len() {
        let _ = text_blob(&good[..n]);
        for x in [0u8, 0x7f, 0xff] {
            let mut bad = good.clone();
            bad[n] = x;
            let _ = text_blob(&bad);
        }
    }
    let z = zip(&[("definition.json", AE_DEFINITION.as_bytes())], true);
    for n in 0..z.len() {
        let _ = import(&z[..n], "x");
        let mut bad = z.clone();
        bad[n] ^= 0xff;
        let _ = import(&bad, "x");
    }
}

/// Real text styles: `OVERFILM_PRTEXTSTYLES=<dir of .prtextstyle files> cargo test -p overfilm-captions -- --ignored real_text_styles --nocapture`.
#[test]
#[ignore]
fn real_text_styles() {
    let Ok(dir) = std::env::var("OVERFILM_PRTEXTSTYLES") else { return };
    let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let bytes = std::fs::read(&p).unwrap();
        match import(&bytes, &p.file_stem().unwrap().to_string_lossy()) {
            Ok(r) => {
                for (preset, font) in r.presets.iter().zip(&r.fonts) {
                    println!(
                        "{}: {} | {font} {} | fill {:?} | stroke {:?} | box {:?} | shadow {} | motion {:?}",
                        p.display(),
                        preset.name,
                        preset.size,
                        preset.fill,
                        preset.stroke,
                        preset.boxed,
                        preset.shadow,
                        preset.motion_in
                    );
                }
            }
            Err(e) => println!("{}: {e}", p.display()),
        }
    }
}

/// The engine's tests use this text style as base64 (`overfilm_captions.rs`).
const YELLOW_BOXED_B64: &str = "RAEAAAAAAABEMyIRDAAAAAYACAAEAAAACAAAAFAAAABKACQABAAIAAAAAAAAAAAAAAAAAAAAAAAAAAwAAAAAAAAAAAAAABAAFAAYABwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAAAATAAAACAAAACgAAAAAQAAAMQAAAABAAAAAABwQgAAkEEAABBBAQAAAAwAAAAIAAwABAAIAAgAAAAIAAAAIAAAAAIAAABBYQAAEgAYAAAABAAIAAAADAAQABQAAAAUAAAAAACQQhwAAAAwAAAAAQAAAAAAAEEKAAwAAAAEAAgAAAAMAAAA3AAAAAAAAAAKABAABAAIAAwAAAAMAAAAAAAAAAAAAAAAAAAAAQAAAAQAAAAVAAAATmFudW1TcXVhcmUtRXh0cmFCb2xkAAoAEAAEAAgADAAKAAAAAAAAAAAAAAAAAAAA";

#[test]
fn the_engine_fixture_is_this_style() {
    assert_eq!(b64(&yellow_boxed()), YELLOW_BOXED_B64);
}
