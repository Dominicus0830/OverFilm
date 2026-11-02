//! A small After Effects expression evaluator (milestone M4.3), enough for the expressions
//! templates put on a caption: arithmetic on numbers and arrays, variables, `if` / `else`, the
//! ternary, `time`, `value`, `ease` / `easeIn` / `easeOut` / `linear`, `clamp`, `Math.*`,
//! expression controls (`effect("Duration")("Slider")`, also through
//! `thisComp.layer("Controls")`), other layers' transform values, `valueAtTime`, `loopOut` /
//! `loopIn`, and `wiggle` (reported to the caller, which turns it into OverFilm's Wiggle effect,
//! the value itself staying still).
//!
//! Anything else (loops, functions, `sourceRectAtTime`, random numbers …) is an error, and the
//! caller keeps the property's keyframes. Evaluation is bounded: a step budget, a nesting
//! limit and capped sizes, so a hostile expression can never hang or crash.

use std::collections::BTreeMap;

const MAX_STEPS: u32 = 20_000;
const MAX_NEST: u32 = 48;
const MAX_TOKENS: usize = 8_000;
const MAX_ARRAY: usize = 64;

/// What an expression can ask about the project.
pub(crate) trait Host {
    /// The property's own value (keyframes or still value, no expression) at `t` seconds.
    fn own_value(&self, t: f64) -> Vec<f64>;
    /// The first and last keyframe times of the property.
    fn own_keys(&self) -> Option<(f64, f64)>;
    /// A layer of the expression's composition by name or 1-based index.
    fn layer(&self, by: &Name) -> Option<usize>;
    fn this_layer(&self) -> usize;
    /// A transform value of a layer (`position`, `scale` in %, `rotation`, `opacity` in %,
    /// `anchorPoint`, `xPosition`, `yPosition`) at `t`.
    fn transform(&self, layer: usize, prop: &str, t: f64) -> Option<Vec<f64>>;
    /// An effect parameter (by effect name or index, parameter name or index) at `t`.
    fn effect(&self, layer: usize, effect: &Name, param: &Name, t: f64) -> Option<Vec<f64>>;
    fn layer_times(&self, layer: usize) -> (f64, f64);
    /// The composition's duration, width, height and frame duration.
    fn comp(&self) -> (f64, f64, f64, f64);
}

/// A name or a 1-based index.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Name {
    Text(String),
    Index(usize),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Id(String),
    Op(&'static str),
}

const OPS: &[&str] = &[
    "===", "!==", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "++", "--", "+", "-", "*", "/", "%", "<", ">", "!", "?", ":", "=", "(", ")", "[",
    "]", "{", "}", ",", ";", ".",
];

fn lex(src: &str) -> Option<Vec<Tok>> {
    let s: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    let mut out = Vec::new();
    while let Some(&c) = s.get(i) {
        if out.len() > MAX_TOKENS {
            return None;
        }
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && s.get(i + 1) == Some(&'/') {
            while s.get(i).is_some_and(|c| *c != '\n') {
                i += 1;
            }
        } else if c == '/' && s.get(i + 1) == Some(&'*') {
            i += 2;
            while s.get(i).is_some() && !(s.get(i) == Some(&'*') && s.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if c.is_ascii_digit() || (c == '.' && s.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let start = i;
            while s.get(i).is_some_and(|c| c.is_ascii_digit() || *c == '.') {
                i += 1;
            }
            if s.get(i).is_some_and(|c| *c == 'e' || *c == 'E') {
                i += 1;
                if s.get(i).is_some_and(|c| *c == '-' || *c == '+') {
                    i += 1;
                }
                while s.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
            }
            let text: String = s.get(start..i)?.iter().collect();
            out.push(Tok::Num(text.parse().ok()?));
        } else if c == '"' || c == '\'' {
            i += 1;
            let mut text = String::new();
            while let Some(&d) = s.get(i) {
                i += 1;
                if d == c {
                    break;
                }
                if d == '\\' {
                    if let Some(&e) = s.get(i) {
                        text.push(match e {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                        i += 1;
                    }
                } else {
                    text.push(d);
                }
                if text.len() > 400 {
                    return None;
                }
            }
            out.push(Tok::Str(text));
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while s.get(i).is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '$') {
                i += 1;
            }
            out.push(Tok::Id(s.get(start..i)?.iter().collect()));
        } else {
            let rest: String = s.get(i..(i + 3).min(s.len()))?.iter().collect();
            let op = OPS.iter().find(|o| rest.starts_with(**o))?;
            i += op.chars().count();
            out.push(Tok::Op(op));
        }
    }
    Some(out)
}

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Str(String),
    Id(String),
    Arr(Vec<Expr>),
    Unary(&'static str, Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Assign(String, &'static str, Box<Expr>),
}

#[derive(Clone, Debug)]
enum Stmt {
    Expr(Expr),
    Var(Vec<(String, Option<Expr>)>),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
    nest: u32,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(o)) if *o == op)
    }
    fn eat(&mut self, op: &str) -> bool {
        let yes = self.is_op(op);
        if yes {
            self.i += 1;
        }
        yes
    }
    fn expect(&mut self, op: &str) -> Option<()> {
        self.eat(op).then_some(())
    }
    fn deeper(&mut self) -> Option<()> {
        self.nest += 1;
        (self.nest <= MAX_NEST).then_some(())
    }

    fn program(&mut self) -> Option<Vec<Stmt>> {
        let mut out = Vec::new();
        while self.peek().is_some() {
            if self.eat(";") {
                continue;
            }
            out.push(self.stmt()?);
        }
        Some(out)
    }

    fn block(&mut self) -> Option<Vec<Stmt>> {
        if self.eat("{") {
            let mut out = Vec::new();
            while !self.eat("}") {
                self.peek()?;
                if self.eat(";") {
                    continue;
                }
                out.push(self.stmt()?);
            }
            Some(out)
        } else {
            Some(vec![self.stmt()?])
        }
    }

    fn stmt(&mut self) -> Option<Stmt> {
        self.deeper()?;
        let s = match self.peek() {
            Some(Tok::Id(k)) if k == "var" || k == "let" || k == "const" => {
                self.i += 1;
                let mut vars = Vec::new();
                loop {
                    let Some(Tok::Id(name)) = self.peek().cloned() else { return None };
                    self.i += 1;
                    let init = if self.eat("=") { Some(self.expr()?) } else { None };
                    vars.push((name, init));
                    if !self.eat(",") {
                        break;
                    }
                }
                Stmt::Var(vars)
            }
            Some(Tok::Id(k)) if k == "if" => {
                self.i += 1;
                self.expect("(")?;
                let c = self.expr()?;
                self.expect(")")?;
                let a = self.block()?;
                let b = if matches!(self.peek(), Some(Tok::Id(k)) if k == "else") {
                    self.i += 1;
                    self.block()?
                } else {
                    Vec::new()
                };
                Stmt::If(c, a, b)
            }
            Some(Tok::Id(k)) if matches!(k.as_str(), "for" | "while" | "do" | "function" | "return" | "switch" | "try" | "new" | "class") => return None,
            _ => Stmt::Expr(self.expr()?),
        };
        self.nest -= 1;
        self.eat(";");
        Some(s)
    }

    fn expr(&mut self) -> Option<Expr> {
        self.deeper()?;
        let e = self.assign()?;
        self.nest -= 1;
        Some(e)
    }

    fn assign(&mut self) -> Option<Expr> {
        let lhs = self.cond()?;
        for op in ["=", "+=", "-=", "*=", "/="] {
            if self.eat(op) {
                let Expr::Id(name) = lhs else { return None };
                let rhs = self.assign()?;
                return Some(Expr::Assign(name, op, Box::new(rhs)));
            }
        }
        Some(lhs)
    }

    fn cond(&mut self) -> Option<Expr> {
        let c = self.binary(0)?;
        if self.eat("?") {
            let a = self.assign()?;
            self.expect(":")?;
            let b = self.assign()?;
            return Some(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Some(c)
    }

    fn binary(&mut self, level: usize) -> Option<Expr> {
        const LEVELS: &[&[&str]] = &[&["||"], &["&&"], &["==", "!=", "===", "!=="], &["<", ">", "<=", ">="], &["+", "-"], &["*", "/", "%"]];
        let Some(ops) = LEVELS.get(level) else { return self.unary() };
        let mut lhs = self.binary(level + 1)?;
        while let Some(Tok::Op(op)) = self.peek().cloned() {
            if !ops.contains(&op) {
                break;
            }
            self.i += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Some(lhs)
    }

    fn unary(&mut self) -> Option<Expr> {
        self.deeper()?;
        let e = if self.eat("-") {
            Expr::Unary("-", Box::new(self.unary()?))
        } else if self.eat("+") {
            self.unary()?
        } else if self.eat("!") {
            Expr::Unary("!", Box::new(self.unary()?))
        } else {
            self.postfix()?
        };
        self.nest -= 1;
        Some(e)
    }

    fn postfix(&mut self) -> Option<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat("(") {
                let mut args = Vec::new();
                if !self.eat(")") {
                    loop {
                        args.push(self.expr()?);
                        if self.eat(")") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                e = Expr::Call(Box::new(e), args);
            } else if self.eat(".") {
                let Some(Tok::Id(name)) = self.peek().cloned() else { return None };
                self.i += 1;
                e = Expr::Member(Box::new(e), name);
            } else if self.eat("[") {
                let ix = self.expr()?;
                self.expect("]")?;
                e = Expr::Index(Box::new(e), Box::new(ix));
            } else {
                break;
            }
        }
        Some(e)
    }

    fn primary(&mut self) -> Option<Expr> {
        let tok = self.peek().cloned()?;
        self.i += 1;
        match tok {
            Tok::Num(n) => Some(Expr::Num(n)),
            Tok::Str(s) => Some(Expr::Str(s)),
            Tok::Id(id) => match id.as_str() {
                "true" => Some(Expr::Num(1.0)),
                "false" => Some(Expr::Num(0.0)),
                _ => Some(Expr::Id(id)),
            },
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                Some(e)
            }
            Tok::Op("[") => {
                let mut items = Vec::new();
                if !self.eat("]") {
                    loop {
                        items.push(self.expr()?);
                        if self.eat("]") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Some(Expr::Arr(items))
            }
            _ => None,
        }
    }
}

/// A parsed expression.
#[derive(Clone, Debug)]
pub(crate) struct Program(Vec<Stmt>);

pub(crate) fn parse(src: &str) -> Option<Program> {
    let t = lex(src)?;
    let mut p = Parser { t, i: 0, nest: 0 };
    let prog = p.program()?;
    (!prog.is_empty()).then_some(Program(prog))
}

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Num(f64),
    Arr(Vec<f64>),
    Str(String),
    Layer(usize),
    Effect(usize, Name),
    Transform(usize),
    Prop(usize, String),
    Comp,
    Math,
    Own,
}

/// A `wiggle(frequency, amount)` met while evaluating.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Wiggle {
    pub freq: f64,
    pub amount: f64,
}

struct Eval<'h, H: Host> {
    h: &'h H,
    t: f64,
    vars: BTreeMap<String, Val>,
    steps: u32,
    wiggle: Option<Wiggle>,
}

fn nums(v: &Val) -> Option<Vec<f64>> {
    match v {
        Val::Num(n) => Some(vec![*n]),
        Val::Arr(a) => Some(a.clone()),
        _ => None,
    }
}

fn from_vec(v: Vec<f64>) -> Val {
    if v.len() == 1 { Val::Num(v.first().copied().unwrap_or(0.0)) } else { Val::Arr(v) }
}

fn ease_curve(kind: &str, s: f64) -> f64 {
    let s = s.clamp(0.0, 1.0);
    match kind {
        "linear" => s,
        "easeIn" => s * s * (2.0 - s),
        "easeOut" => 1.0 - (1.0 - s) * (1.0 - s) * (1.0 + s),
        _ => s * s * (3.0 - 2.0 * s),
    }
}

impl<H: Host> Eval<'_, H> {
    fn step(&mut self) -> Option<()> {
        self.steps += 1;
        (self.steps <= MAX_STEPS).then_some(())
    }

    /// A value made concrete (properties read at the current time).
    fn concrete(&mut self, v: Val) -> Option<Val> {
        Some(match v {
            Val::Prop(l, p) => from_vec(self.h.transform(l, &p, self.t)?),
            Val::Own => from_vec(self.h.own_value(self.t)),
            other => other,
        })
    }

    fn run(&mut self, prog: &[Stmt]) -> Option<Option<Val>> {
        let mut last = None;
        for s in prog {
            self.step()?;
            match s {
                Stmt::Expr(e) => last = Some(self.eval(e)?),
                Stmt::Var(vars) => {
                    for (name, init) in vars {
                        let v = match init {
                            Some(e) => self.eval(e)?,
                            None => Val::Num(0.0),
                        };
                        if self.vars.len() > 256 {
                            return None;
                        }
                        self.vars.insert(name.clone(), v.clone());
                        last = Some(v);
                    }
                }
                Stmt::If(c, a, b) => {
                    let c = self.eval(c)?;
                    let branch = if self.truthy(&c) { a } else { b };
                    if let Some(v) = self.run(branch)? {
                        last = Some(v);
                    }
                }
            }
        }
        Some(last)
    }

    fn truthy(&self, v: &Val) -> bool {
        match v {
            Val::Num(n) => *n != 0.0 && !n.is_nan(),
            Val::Str(s) => !s.is_empty(),
            _ => true,
        }
    }

    fn eval(&mut self, e: &Expr) -> Option<Val> {
        self.step()?;
        let v = match e {
            Expr::Num(n) => Val::Num(*n),
            Expr::Str(s) => Val::Str(s.clone()),
            Expr::Arr(items) => {
                let mut out = Vec::new();
                for it in items.iter().take(MAX_ARRAY) {
                    let v = self.eval(it)?;
                    out.extend(nums(&v)?);
                }
                Val::Arr(out)
            }
            Expr::Id(name) => self.ident(name)?,
            Expr::Unary(op, a) => {
                let a = self.eval(a)?;
                match *op {
                    "-" => from_vec(nums(&a)?.into_iter().map(|x| -x).collect()),
                    _ => Val::Num(if self.truthy(&a) { 0.0 } else { 1.0 }),
                }
            }
            Expr::Bin(op, a, b) => {
                if *op == "&&" || *op == "||" {
                    let a = self.eval(a)?;
                    let ta = self.truthy(&a);
                    return if (*op == "&&") == ta { self.eval(b) } else { Some(a) };
                }
                let a = self.eval(a)?;
                let b = self.eval(b)?;
                self.binop(op, a, b)?
            }
            Expr::Cond(c, a, b) => {
                let c = self.eval(c)?;
                if self.truthy(&c) { self.eval(a)? } else { self.eval(b)? }
            }
            Expr::Assign(name, op, rhs) => {
                let r = self.eval(rhs)?;
                let v = match *op {
                    "=" => r,
                    other => {
                        let cur = self.vars.get(name).cloned()?;
                        self.binop(other.trim_end_matches('='), cur, r)?
                    }
                };
                if self.vars.len() > 256 {
                    return None;
                }
                self.vars.insert(name.clone(), v.clone());
                v
            }
            Expr::Member(obj, name) => {
                let o = self.eval(obj)?;
                self.member(o, name)?
            }
            Expr::Index(obj, ix) => {
                let o = self.eval(obj)?;
                let o = self.concrete(o)?;
                let i = nums(&self.eval(ix)?)?.first().copied()?;
                let a = nums(&o)?;
                Val::Num(a.get(i.max(0.0) as usize).copied()?)
            }
            Expr::Call(callee, args) => self.call(callee, args)?,
        };
        Some(v)
    }

    fn ident(&mut self, name: &str) -> Option<Val> {
        if let Some(v) = self.vars.get(name) {
            return Some(v.clone());
        }
        let this = self.h.this_layer();
        Some(match name {
            "time" => Val::Num(self.t),
            "value" => Val::Own,
            "thisComp" => Val::Comp,
            "thisLayer" => Val::Layer(this),
            "thisProperty" => Val::Own,
            "Math" => Val::Math,
            "transform" => Val::Transform(this),
            "position" | "scale" | "rotation" | "opacity" | "anchorPoint" | "xPosition" | "yPosition" => Val::Prop(this, name.to_string()),
            "inPoint" => Val::Num(self.h.layer_times(this).0),
            "outPoint" => Val::Num(self.h.layer_times(this).1),
            _ => return None,
        })
    }

    fn member(&mut self, o: Val, name: &str) -> Option<Val> {
        Some(match (o, name) {
            (Val::Layer(l), "transform") => Val::Transform(l),
            (Val::Layer(l) | Val::Transform(l), p @ ("position" | "scale" | "rotation" | "opacity" | "anchorPoint" | "xPosition" | "yPosition")) => {
                Val::Prop(l, p.to_string())
            }
            (Val::Transform(l), "zRotation") => Val::Prop(l, "rotation".into()),
            (Val::Layer(l), "inPoint") => Val::Num(self.h.layer_times(l).0),
            (Val::Layer(l), "outPoint") => Val::Num(self.h.layer_times(l).1),
            (Val::Comp, "duration") => Val::Num(self.h.comp().0),
            (Val::Comp, "width") => Val::Num(self.h.comp().1),
            (Val::Comp, "height") => Val::Num(self.h.comp().2),
            (Val::Comp, "frameDuration") => Val::Num(self.h.comp().3),
            (Val::Math, "PI") => Val::Num(std::f64::consts::PI),
            (Val::Math, "E") => Val::Num(std::f64::consts::E),
            (v @ (Val::Prop(..) | Val::Own), "value") => self.concrete(v)?,
            (v, "length") => {
                let v = self.concrete(v)?;
                match v {
                    Val::Arr(a) => Val::Num(a.len() as f64),
                    Val::Str(s) => Val::Num(s.chars().count() as f64),
                    _ => return None,
                }
            }
            (Val::Arr(a), "x") => Val::Num(a.first().copied()?),
            (Val::Arr(a), "y") => Val::Num(a.get(1).copied()?),
            _ => return None,
        })
    }

    fn args(&mut self, args: &[Expr]) -> Option<Vec<Val>> {
        let mut out = Vec::new();
        for a in args.iter().take(8) {
            let v = self.eval(a)?;
            out.push(self.concrete(v)?);
        }
        Some(out)
    }

    fn num_arg(v: Option<&Val>) -> Option<f64> {
        nums(v?)?.first().copied()
    }

    fn name_arg(v: Option<&Val>) -> Option<Name> {
        match v? {
            Val::Str(s) => Some(Name::Text(s.clone())),
            Val::Num(n) if *n >= 1.0 && *n < 1e6 => Some(Name::Index(*n as usize)),
            _ => None,
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr]) -> Option<Val> {
        match callee {
            Expr::Id(f) if !self.vars.contains_key(f) => self.global(f, args),
            Expr::Member(obj, m) => {
                let o = self.eval(obj)?;
                let a = self.args(args)?;
                self.method(o, m, &a)
            }
            other => {
                // effect("Name")("Slider")
                let f = self.eval(other)?;
                let a = self.args(args)?;
                match f {
                    Val::Effect(l, e) => Some(from_vec(self.h.effect(l, &e, &Self::name_arg(a.first())?, self.t)?)),
                    _ => None,
                }
            }
        }
    }

    fn method(&mut self, o: Val, m: &str, a: &[Val]) -> Option<Val> {
        Some(match (o, m) {
            (Val::Comp, "layer") => Val::Layer(self.h.layer(&Self::name_arg(a.first())?)?),
            (Val::Layer(l), "effect") => Val::Effect(l, Self::name_arg(a.first())?),
            (Val::Prop(l, p), "valueAtTime") => from_vec(self.h.transform(l, &p, Self::num_arg(a.first())?)?),
            (Val::Own, "valueAtTime") => from_vec(self.h.own_value(Self::num_arg(a.first())?)),
            (Val::Math, f) => {
                let x = Self::num_arg(a.first()).unwrap_or(0.0);
                let y = Self::num_arg(a.get(1)).unwrap_or(0.0);
                Val::Num(match f {
                    "sin" => x.sin(),
                    "cos" => x.cos(),
                    "tan" => x.tan(),
                    "asin" => x.asin(),
                    "acos" => x.acos(),
                    "atan" => x.atan(),
                    "atan2" => x.atan2(y),
                    "abs" => x.abs(),
                    "sqrt" => x.sqrt(),
                    "exp" => x.exp(),
                    "log" => x.ln(),
                    "pow" => x.powf(y),
                    "floor" => x.floor(),
                    "ceil" => x.ceil(),
                    "round" => x.round(),
                    "min" => a.iter().filter_map(|v| Self::num_arg(Some(v))).fold(f64::INFINITY, f64::min),
                    "max" => a.iter().filter_map(|v| Self::num_arg(Some(v))).fold(f64::NEG_INFINITY, f64::max),
                    _ => return None,
                })
            }
            (v @ (Val::Prop(..) | Val::Own), "wiggle") => {
                self.note_wiggle(a);
                self.concrete(v)?
            }
            _ => return None,
        })
    }

    fn note_wiggle(&mut self, a: &[Val]) {
        if let (Some(freq), Some(amount)) = (Self::num_arg(a.first()), Self::num_arg(a.get(1))) {
            self.wiggle = Some(Wiggle { freq, amount });
        }
    }

    fn global(&mut self, f: &str, args: &[Expr]) -> Option<Val> {
        let a = self.args(args)?;
        let this = self.h.this_layer();
        Some(match f {
            "ease" | "easeIn" | "easeOut" | "linear" => {
                let (t, t0, t1, v0, v1) = match a.len() {
                    3 => (Self::num_arg(a.first())?, 0.0, 1.0, a.get(1)?, a.get(2)?),
                    5 => (Self::num_arg(a.first())?, Self::num_arg(a.get(1))?, Self::num_arg(a.get(2))?, a.get(3)?, a.get(4)?),
                    _ => return None,
                };
                let s = if (t1 - t0).abs() < 1e-12 { if t < t0 { 0.0 } else { 1.0 } } else { (t - t0) / (t1 - t0) };
                let k = ease_curve(f, s);
                let (v0, v1) = (nums(v0)?, nums(v1)?);
                from_vec(v0.iter().zip(v1.iter().chain(std::iter::repeat(&0.0))).map(|(x, y)| x + (y - x) * k).collect())
            }
            "clamp" => {
                let v = nums(a.first()?)?;
                let (lo, hi) = (nums(a.get(1)?)?, nums(a.get(2)?)?);
                from_vec(
                    v.iter()
                        .enumerate()
                        .map(|(i, x)| {
                            let l = lo.get(i).or(lo.first()).copied().unwrap_or(f64::NEG_INFINITY);
                            let h = hi.get(i).or(hi.first()).copied().unwrap_or(f64::INFINITY);
                            if l <= h { x.clamp(l, h) } else { *x }
                        })
                        .collect(),
                )
            }
            "effect" => Val::Effect(this, Self::name_arg(a.first())?),
            "wiggle" => {
                self.note_wiggle(&a);
                from_vec(self.h.own_value(self.t))
            }
            "loopOut" | "loopIn" | "loopOutDuration" | "loopInDuration" => {
                let kind = match a.first() {
                    Some(Val::Str(s)) => s.to_ascii_lowercase(),
                    _ => "cycle".into(),
                };
                from_vec(self.looped(f.starts_with("loopOut"), &kind))
            }
            "degreesToRadians" => Val::Num(Self::num_arg(a.first())?.to_radians()),
            "radiansToDegrees" => Val::Num(Self::num_arg(a.first())?.to_degrees()),
            "framesToTime" => Val::Num(Self::num_arg(a.first())? * self.h.comp().3),
            "timeToFrames" => Val::Num(Self::num_arg(a.first()).unwrap_or(self.t) / self.h.comp().3.max(1e-6)),
            "add" => self.binop("+", a.first()?.clone(), a.get(1)?.clone())?,
            "sub" => self.binop("-", a.first()?.clone(), a.get(1)?.clone())?,
            "mul" => self.binop("*", a.first()?.clone(), a.get(1)?.clone())?,
            "div" => self.binop("/", a.first()?.clone(), a.get(1)?.clone())?,
            "length" => {
                let v = nums(a.first()?)?;
                let w = a.get(1).and_then(nums).unwrap_or_else(|| vec![0.0; v.len()]);
                Val::Num(v.iter().zip(w.iter()).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt())
            }
            "Number" | "parseFloat" => Val::Num(match a.first()? {
                Val::Str(s) => s.trim().parse().ok()?,
                other => Self::num_arg(Some(other))?,
            }),
            "posterizeTime" => Val::Num(0.0),
            _ => return None,
        })
    }

    /// `loopOut` / `loopIn` of the property's own keyframes.
    fn looped(&self, out: bool, kind: &str) -> Vec<f64> {
        let Some((a, b)) = self.h.own_keys() else { return self.h.own_value(self.t) };
        let span = b - a;
        if span <= 1e-9 || (out && self.t <= b) || (!out && self.t >= a) {
            return self.h.own_value(self.t);
        }
        let off = if out { self.t - a } else { b - self.t };
        let cycles = (off / span).floor();
        let within = off - cycles * span;
        let at = match kind {
            "pingpong" if (cycles as i64) % 2 == 1 => {
                if out {
                    b - within
                } else {
                    a + within
                }
            }
            "continue" => {
                return self.h.own_value(if out { b } else { a });
            }
            _ => {
                if out {
                    a + within
                } else {
                    b - within
                }
            }
        };
        let mut v = self.h.own_value(at);
        if kind == "offset" {
            let (va, vb) = (self.h.own_value(a), self.h.own_value(b));
            let k = if out { cycles } else { -cycles };
            for (x, (p, q)) in v.iter_mut().zip(va.iter().zip(vb.iter())) {
                *x += (q - p) * k;
            }
        }
        v
    }

    fn binop(&mut self, op: &str, a: Val, b: Val) -> Option<Val> {
        let a = self.concrete(a)?;
        let b = self.concrete(b)?;
        if let (Val::Str(x), Val::Str(y)) = (&a, &b) {
            return Some(match op {
                "+" => Val::Str(format!("{x}{y}").chars().take(400).collect()),
                "==" | "===" => Val::Num(f64::from(u8::from(x == y))),
                "!=" | "!==" => Val::Num(f64::from(u8::from(x != y))),
                _ => return None,
            });
        }
        let (x, y) = (nums(&a)?, nums(&b)?);
        let scalar = |v: &[f64]| (v.len() == 1).then(|| v.first().copied().unwrap_or(0.0));
        let both = |f: fn(f64, f64) -> f64| -> Option<Val> {
            match (scalar(&x), scalar(&y)) {
                (Some(p), Some(q)) => Some(Val::Num(f(p, q))),
                (None, Some(q)) => Some(Val::Arr(x.iter().map(|p| f(*p, q)).collect())),
                (Some(p), None) => Some(Val::Arr(y.iter().map(|q| f(p, *q)).collect())),
                (None, None) => {
                    let n = x.len().max(y.len());
                    Some(Val::Arr((0..n).map(|i| f(x.get(i).copied().unwrap_or(0.0), y.get(i).copied().unwrap_or(0.0))).collect()))
                }
            }
        };
        let cmp = |f: fn(f64, f64) -> bool| -> Option<Val> { Some(Val::Num(f64::from(u8::from(f(scalar(&x)?, scalar(&y)?))))) };
        match op {
            "+" => both(|p, q| p + q),
            "-" => both(|p, q| p - q),
            "*" => both(|p, q| p * q),
            "/" => both(|p, q| if q == 0.0 { 0.0 } else { p / q }),
            "%" => both(|p, q| if q == 0.0 { 0.0 } else { p % q }),
            "<" => cmp(|p, q| p < q),
            ">" => cmp(|p, q| p > q),
            "<=" => cmp(|p, q| p <= q),
            ">=" => cmp(|p, q| p >= q),
            "==" | "===" => Some(Val::Num(f64::from(u8::from(x == y)))),
            "!=" | "!==" => Some(Val::Num(f64::from(u8::from(x != y)))),
            _ => None,
        }
    }
}

/// The value of `prog` at `t` seconds (layer time), and any `wiggle` it asked for. `None` when
/// the expression uses something this evaluator does not know.
pub(crate) fn eval<H: Host>(prog: &Program, h: &H, t: f64) -> Option<(Vec<f64>, Option<Wiggle>)> {
    let mut e = Eval { h, t, vars: BTreeMap::new(), steps: 0, wiggle: None };
    let last = e.run(&prog.0)??;
    let v = e.concrete(last)?;
    let out = nums(&v)?;
    (!out.is_empty() && out.iter().all(|x| x.is_finite())).then_some((out, e.wiggle))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct H;
    impl Host for H {
        fn own_value(&self, t: f64) -> Vec<f64> {
            vec![t * 10.0, 5.0]
        }
        fn own_keys(&self) -> Option<(f64, f64)> {
            Some((0.0, 1.0))
        }
        fn layer(&self, by: &Name) -> Option<usize> {
            (by == &Name::Text("Controls".into())).then_some(1)
        }
        fn this_layer(&self) -> usize {
            0
        }
        fn transform(&self, layer: usize, prop: &str, _t: f64) -> Option<Vec<f64>> {
            (layer == 1 && prop == "scale").then(|| vec![50.0, 50.0])
        }
        fn effect(&self, layer: usize, effect: &Name, param: &Name, _t: f64) -> Option<Vec<f64>> {
            (layer == 1 && effect == &Name::Text("Dur".into()) && (param == &Name::Text("Slider".into()) || param == &Name::Index(1))).then(|| vec![2.0])
        }
        fn layer_times(&self, _layer: usize) -> (f64, f64) {
            (0.5, 4.0)
        }
        fn comp(&self) -> (f64, f64, f64, f64) {
            (4.0, 1920.0, 1080.0, 1.0 / 30.0)
        }
    }

    fn at(src: &str, t: f64) -> Option<Vec<f64>> {
        eval(&parse(src)?, &H, t).map(|(v, _)| v)
    }

    #[test]
    fn template_expressions_evaluate() {
        let src = "dur = thisComp.layer(\"Controls\").effect(\"Dur\")(\"Slider\");\r\nease(time, 0, dur, -60, 100)\r\n";
        assert_eq!(at(src, 0.0), Some(vec![-60.0]));
        assert_eq!(at(src, 2.0), Some(vec![100.0]));
        let mid = at(src, 1.0).unwrap()[0];
        assert!((mid - 20.0).abs() < 1e-9, "{mid}");
        // arrays, variables, if/else, comments, Math
        let src = "// slide\nvar startPos = [2000, 1920], endPos = [1080, 1920];\nt = time - inPoint;\nif (t < 0) { startPos } else { linear(t, 0, 1, startPos, endPos) + [0, Math.round(0.4)] }";
        assert_eq!(at(src, 0.0), Some(vec![2000.0, 1920.0]));
        assert_eq!(at(src, 1.0), Some(vec![1540.0, 1920.0]));
        assert_eq!(at("value + [1, 1]", 2.0), Some(vec![21.0, 6.0]));
        assert_eq!(at("thisComp.layer(\"Controls\").transform.scale[0] / 100", 0.0), Some(vec![0.5]));
        assert_eq!(at("effect(\"Dur\")(1) * 3", 0.0), None, "this layer has no such effect");
        assert_eq!(at("x = 3; x *= 2; x > 5 ? 1 : 0", 0.0), Some(vec![1.0]));
        assert_eq!(at("clamp(time * 100, 0, 50)", 1.0), Some(vec![50.0]));
    }

    #[test]
    fn wiggle_and_loops() {
        let (v, w) = eval(&parse("wiggle(3, 25)").unwrap(), &H, 0.5).unwrap();
        assert_eq!(v, vec![5.0, 5.0]);
        assert_eq!(w, Some(Wiggle { freq: 3.0, amount: 25.0 }));
        assert_eq!(at("loopOut(\"cycle\")", 1.25), Some(vec![2.5, 5.0]));
        assert_eq!(at("loopOut()", 0.5), Some(vec![5.0, 5.0]));
    }

    #[test]
    fn unknown_or_hostile_expressions_are_refused() {
        for src in [
            "thisComp.layer(\"TXT\").sourceRectAtTime(time,false).width",
            "for (i = 0; i < 10; i++) {}",
            "while(1){}",
            "function f(){ return 1 }",
            "random(10)",
            "nope",
            "",
            "((((((((((((((((((((((((((((((((((((((((((((((((((((((((((1))))))))))))))))))))))))))))))))))))))))))))))))))))))))))",
            "\"unterminated",
            "1 +",
            "[1,2](3)",
        ] {
            assert_eq!(parse(src).and_then(|p| eval(&p, &H, 0.0)).map(|x| x.0), None, "{src}");
        }
        // a long chain stays within the step budget and fails cleanly
        let long = "x = 1;".repeat(30_000);
        assert_eq!(parse(&long).and_then(|p| eval(&p, &H, 0.0)), None);
    }
}
