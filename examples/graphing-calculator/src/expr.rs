//! Expressions: parsed once into a tree, then evaluated as often as a plot needs.
//!
//! Numbers (`2.5`, `1e-3`), `x` and `y`, `ans`, `π`/`pi`, `e`; `+ - × ÷ * / ^`, postfix `!`
//! (factorial), `%` (percent) and `²`; `√`; functions `sin cos tan asin acos atan sinh
//! cosh tanh sqrt cbrt ln log log2 exp abs floor ceil round sign min max`. Implicit
//! multiplication (`2x`, `3sin(x)`, `2π`, `(x+1)(x-1)`), and a function's argument
//! without brackets (`sin 30`).

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    X,
    Y,
    Ans,
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
    Factorial(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Sqrt,
    Cbrt,
    Ln,
    Log,
    Log2,
    Exp,
    Abs,
    Floor,
    Ceil,
    Round,
    Sign,
    Min,
    Max,
}

const FUNCS: &[(&str, Func, usize)] = &[
    ("sin", Func::Sin, 1),
    ("cos", Func::Cos, 1),
    ("tan", Func::Tan, 1),
    ("asin", Func::Asin, 1),
    ("acos", Func::Acos, 1),
    ("atan", Func::Atan, 1),
    ("sinh", Func::Sinh, 1),
    ("cosh", Func::Cosh, 1),
    ("tanh", Func::Tanh, 1),
    ("sqrt", Func::Sqrt, 1),
    ("cbrt", Func::Cbrt, 1),
    ("ln", Func::Ln, 1),
    ("log", Func::Log, 1),
    ("log2", Func::Log2, 1),
    ("exp", Func::Exp, 1),
    ("abs", Func::Abs, 1),
    ("floor", Func::Floor, 1),
    ("ceil", Func::Ceil, 1),
    ("round", Func::Round, 1),
    ("sign", Func::Sign, 1),
    ("min", Func::Min, 2),
    ("max", Func::Max, 2),
];

/// What an expression is evaluated with.
#[derive(Debug, Clone, Copy, Default)]
pub struct Env {
    pub x: f64,
    pub y: f64,
    pub ans: f64,
    /// Trigonometry in degrees (the calculator's DEG); plots use radians.
    pub degrees: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Sym(char),
}

fn tokens(src: &str) -> Result<Vec<Tok>, ParseError> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // An exponent: 1e5, 2.5E-3 (but not `2e` alone, which is 2 × e).
            if i + 1 < chars.len()
                && (chars[i] == 'e' || chars[i] == 'E')
                && (chars[i + 1].is_ascii_digit()
                    || ((chars[i + 1] == '-' || chars[i + 1] == '+')
                        && chars.get(i + 2).is_some_and(char::is_ascii_digit)))
            {
                i += 2;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let n = text
                .parse()
                .map_err(|_| ParseError(format!("`{text}` isn't a number")))?;
            out.push(Tok::Num(n));
        } else if c.is_alphabetic() && c != 'π' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() && chars[i] != 'π') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
            out.extend(split_word(&word)?);
        } else {
            let sym = match c {
                '×' | '·' => '*',
                '÷' => '/',
                '−' | '–' => '-',
                c => c,
            };
            if !"+-*/^()!%²√π,|".contains(sym) {
                return Err(ParseError(format!("`{c}` isn't something I can calculate")));
            }
            out.push(Tok::Sym(sym));
            i += 1;
        }
    }
    Ok(out)
}

/// `xsin` → `x`, `sin`; `pi`, `ans`, function names; single letters x, y, e.
fn split_word(word: &str) -> Result<Vec<Tok>, ParseError> {
    let mut out = Vec::new();
    let mut rest = word;
    'outer: while !rest.is_empty() {
        // Longest known name first.
        let mut names: Vec<&str> = FUNCS.iter().map(|f| f.0).collect();
        names.extend(["pi", "ans"]);
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        for n in names {
            if let Some(r) = rest.strip_prefix(n) {
                out.push(Tok::Ident(n.to_owned()));
                rest = r;
                continue 'outer;
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        if matches!(c, 'x' | 'y' | 'e') {
            out.push(Tok::Ident(c.to_string()));
            rest = &rest[c.len_utf8()..];
        } else {
            return Err(ParseError(format!("I don't know `{word}`")));
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::Sym(c)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.term()?;
        loop {
            if self.eat('+') {
                left = Expr::Bin(Op::Add, Box::new(left), Box::new(self.term()?));
            } else if self.eat('-') {
                left = Expr::Bin(Op::Sub, Box::new(left), Box::new(self.term()?));
            } else {
                return Ok(left);
            }
        }
    }
    /// Whether the next token can start a factor: for implicit multiplication.
    fn starts_factor(&self) -> bool {
        match self.peek() {
            Some(Tok::Num(_)) | Some(Tok::Ident(_)) => true,
            Some(Tok::Sym(c)) => matches!(c, '(' | '√' | 'π'),
            None => false,
        }
    }
    fn term(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.unary()?;
        loop {
            if self.eat('*') {
                left = Expr::Bin(Op::Mul, Box::new(left), Box::new(self.unary()?));
            } else if self.eat('/') {
                left = Expr::Bin(Op::Div, Box::new(left), Box::new(self.unary()?));
            } else if self.starts_factor() {
                left = Expr::Bin(Op::Mul, Box::new(left), Box::new(self.power()?));
            } else {
                return Ok(left);
            }
        }
    }
    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat('-') {
            Ok(Expr::Neg(Box::new(self.unary()?)))
        } else if self.eat('+') {
            self.unary()
        } else {
            self.power()
        }
    }
    fn power(&mut self) -> Result<Expr, ParseError> {
        let base = self.postfix()?;
        if self.eat('^') {
            // Right-associative: 2^3^2 = 2^9.
            Ok(Expr::Bin(Op::Pow, Box::new(base), Box::new(self.unary()?)))
        } else {
            Ok(base)
        }
    }
    fn postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.primary()?;
        loop {
            if self.eat('!') {
                e = Expr::Factorial(Box::new(e));
            } else if self.eat('%') {
                e = Expr::Bin(Op::Div, Box::new(e), Box::new(Expr::Num(100.0)));
            } else if self.eat('²') {
                e = Expr::Bin(Op::Pow, Box::new(e), Box::new(Expr::Num(2.0)));
            } else {
                return Ok(e);
            }
        }
    }
    fn primary(&mut self) -> Result<Expr, ParseError> {
        let Some(t) = self.peek().cloned() else {
            return Err(ParseError("It ends too soon".into()));
        };
        self.pos += 1;
        match t {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Sym('(') => {
                let e = self.expr()?;
                if !self.eat(')') {
                    return Err(ParseError("A `(` isn't closed".into()));
                }
                Ok(e)
            }
            Tok::Sym('|') => {
                let e = self.expr()?;
                if !self.eat('|') {
                    return Err(ParseError("A `|` isn't closed".into()));
                }
                Ok(Expr::Call(Func::Abs, vec![e]))
            }
            Tok::Sym('√') => Ok(Expr::Call(Func::Sqrt, vec![self.postfix()?])),
            Tok::Sym('π') => Ok(Expr::Num(std::f64::consts::PI)),
            Tok::Sym(c) => Err(ParseError(format!("`{c}` is in the wrong place"))),
            Tok::Ident(name) => match name.as_str() {
                "x" => Ok(Expr::X),
                "y" => Ok(Expr::Y),
                "e" => Ok(Expr::Num(std::f64::consts::E)),
                "pi" => Ok(Expr::Num(std::f64::consts::PI)),
                "ans" => Ok(Expr::Ans),
                f => {
                    let &(_, func, arity) = FUNCS
                        .iter()
                        .find(|(n, _, _)| *n == f)
                        .ok_or_else(|| ParseError(format!("I don't know `{f}`")))?;
                    let args = if self.eat('(') {
                        let mut args = vec![self.expr()?];
                        while self.eat(',') {
                            args.push(self.expr()?);
                        }
                        if !self.eat(')') {
                            return Err(ParseError(format!("`{f}(` isn't closed")));
                        }
                        args
                    } else {
                        // `sin 30`, `sin x`: the argument without brackets.
                        vec![self.power()?]
                    };
                    if args.len() != arity {
                        return Err(ParseError(format!(
                            "`{f}` takes {arity} value{}",
                            if arity == 1 { "" } else { "s" }
                        )));
                    }
                    Ok(Expr::Call(func, args))
                }
            },
        }
    }
}

pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let toks = tokens(src)?;
    if toks.is_empty() {
        return Err(ParseError("Nothing to calculate".into()));
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.expr()?;
    match p.peek() {
        None => Ok(e),
        Some(Tok::Sym(')')) => Err(ParseError("There's a `)` too many".into())),
        Some(_) => Err(ParseError("Something's missing between the values".into())),
    }
}

fn factorial(n: f64) -> f64 {
    if n < 0.0 || n.fract() != 0.0 || n > 170.0 {
        return f64::NAN;
    }
    (1..=n as u32).fold(1.0, |acc, k| acc * k as f64)
}

impl Expr {
    pub fn eval(&self, env: &Env) -> f64 {
        let to_rad = |a: f64| if env.degrees { a.to_radians() } else { a };
        let from_rad = |a: f64| if env.degrees { a.to_degrees() } else { a };
        match self {
            Expr::Num(n) => *n,
            Expr::X => env.x,
            Expr::Y => env.y,
            Expr::Ans => env.ans,
            Expr::Neg(e) => -e.eval(env),
            Expr::Factorial(e) => factorial(e.eval(env)),
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(env), b.eval(env));
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div => a / b,
                    Op::Pow => a.powf(b),
                }
            }
            Expr::Call(f, args) => {
                let v = args[0].eval(env);
                match f {
                    Func::Sin => snap(to_rad(v).sin()),
                    Func::Cos => snap(to_rad(v).cos()),
                    Func::Tan => {
                        let r = to_rad(v);
                        // tan(90°) is undefined, not 1.6e16.
                        if snap(r.cos()) == 0.0 {
                            f64::NAN
                        } else {
                            snap(r.tan())
                        }
                    }
                    Func::Asin => from_rad(v.asin()),
                    Func::Acos => from_rad(v.acos()),
                    Func::Atan => from_rad(v.atan()),
                    Func::Sinh => v.sinh(),
                    Func::Cosh => v.cosh(),
                    Func::Tanh => v.tanh(),
                    Func::Sqrt => v.sqrt(),
                    Func::Cbrt => v.cbrt(),
                    Func::Ln => v.ln(),
                    Func::Log => v.log10(),
                    Func::Log2 => v.log2(),
                    Func::Exp => v.exp(),
                    Func::Abs => v.abs(),
                    Func::Floor => v.floor(),
                    Func::Ceil => v.ceil(),
                    Func::Round => v.round(),
                    Func::Sign => {
                        if v == 0.0 {
                            0.0
                        } else {
                            v.signum()
                        }
                    }
                    Func::Min => v.min(args[1].eval(env)),
                    Func::Max => v.max(args[1].eval(env)),
                }
            }
        }
    }

    /// Whether `y` appears: a surface z = f(x, y) rather than a curve.
    pub fn uses_y(&self) -> bool {
        self.mentions(&Expr::Y)
    }

    /// Whether `x` appears: something to graph rather than to calculate.
    pub fn uses_x(&self) -> bool {
        self.mentions(&Expr::X)
    }

    fn mentions(&self, var: &Expr) -> bool {
        match self {
            Expr::Neg(e) | Expr::Factorial(e) => e.mentions(var),
            Expr::Bin(_, a, b) => a.mentions(var) || b.mentions(var),
            Expr::Call(_, args) => args.iter().any(|a| a.mentions(var)),
            e => e == var,
        }
    }
}

/// sin(180°) is 0, not 1.2e-16, and sin(30°) is 0.5, not 0.49999999999999994.
fn snap(v: f64) -> f64 {
    if v.abs() < 1e-12 {
        0.0
    } else {
        (v * 1e14).round() / 1e14
    }
}

/// A result as the display shows it: up to 12 significant digits, scientific when huge
/// or tiny.
pub fn format(v: f64) -> String {
    if v.is_nan() {
        return "Not a number".into();
    }
    if v.is_infinite() {
        return if v > 0.0 {
            "∞".into()
        } else {
            "−∞".into()
        };
    }
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    let s = if !(1e-6..1e15).contains(&a) {
        let s = format!("{v:.10e}");
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = m.trim_end_matches('0').trim_end_matches('.');
        format!("{m}×10^{e}")
    } else {
        let digits = 12 - (a.log10().floor() as i32 + 1);
        let s = format!("{v:.*}", digits.clamp(0, 12) as usize);
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s
        }
    };
    s.replace('-', "−")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(s: &str) -> f64 {
        parse(s).unwrap().eval(&Env {
            degrees: true,
            ans: 7.0,
            ..Default::default()
        })
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(calc("1 + 2 × 3"), 7.0);
        assert_eq!(calc("(1 + 2) × 3"), 9.0);
        assert_eq!(calc("2^3^2"), 512.0);
        assert_eq!(calc("-2^2"), -4.0);
        assert_eq!(calc("10 ÷ 4"), 2.5);
        assert_eq!(calc("7 − 10"), -3.0);
        assert_eq!(calc("5!"), 120.0);
        assert_eq!(calc("50%"), 0.5);
        assert_eq!(calc("3²"), 9.0);
        assert_eq!(calc("√16 + |−3|"), 7.0);
        assert_eq!(calc("ans × 2"), 14.0);
        assert_eq!(calc("1.5e3"), 1500.0);
        assert_eq!(calc("2×10^3"), 2000.0);
        assert_eq!(calc("max(2, 9)"), 9.0);
    }

    #[test]
    fn implicit_multiplication_and_constants() {
        assert!((calc("2π") - std::f64::consts::TAU).abs() < 1e-12);
        assert!((calc("2e") - 2.0 * std::f64::consts::E).abs() < 1e-12);
        assert_eq!(calc("(1+1)(2+1)"), 6.0);
        assert_eq!(calc("3sin(90)"), 3.0);
        assert_eq!(calc("sin 30"), 0.5);
        let e = parse("2x + xy").unwrap();
        assert!(e.uses_y());
        assert_eq!(
            e.eval(&Env {
                x: 3.0,
                y: 4.0,
                ..Default::default()
            }),
            18.0
        );
        assert!(!parse("x^2").unwrap().uses_y());
    }

    #[test]
    fn trigonometry_in_degrees_and_radians() {
        assert_eq!(calc("sin(180)"), 0.0);
        assert!((calc("cos(60)") - 0.5).abs() < 1e-12);
        assert!(calc("tan(90)").is_nan());
        assert!((calc("asin(1)") - 90.0).abs() < 1e-9);
        let rad = parse("sin(pi/2)").unwrap().eval(&Env::default());
        assert!((rad - 1.0).abs() < 1e-12);
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert_eq!(parse("(1+2").unwrap_err().0, "A `(` isn't closed");
        assert_eq!(parse("1+2)").unwrap_err().0, "There's a `)` too many");
        assert_eq!(parse("foo(2)").unwrap_err().0, "I don't know `foo`");
        assert_eq!(parse("").unwrap_err().0, "Nothing to calculate");
        assert_eq!(parse("2 +").unwrap_err().0, "It ends too soon");
        assert!(parse("max(1)").is_err());
    }

    #[test]
    fn results_read_well() {
        assert_eq!(format(1024.0), "1024");
        assert_eq!(format(0.1 + 0.2), "0.3");
        assert_eq!(format(-2.5), "−2.5");
        assert_eq!(format(1.0 / 3.0), "0.333333333333");
        assert_eq!(format(6.02e23), "6.02×10^23");
        assert_eq!(format(f64::NAN), "Not a number");
        assert_eq!(format(1.0 / 0.0), "∞");
    }
}
