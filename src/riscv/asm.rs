//! A two-pass assembler for RV32IM + Zicsr in GNU syntax: labels, the usual
//! pseudo-instructions (`li`, `la`, `mv`, `j`, `call`, `ret`, `beqz`, `csrr`,
//! ...), `%hi`/`%lo`, and the directives `.org .align .p2align .balign .word
//! .half .byte .ascii .asciz .string .space .zero .equ .set` (`.text .data
//! .section .globl .global .option .type .size` are accepted and ignored).
//! The program starts at address 0.

use std::collections::HashMap;

use super::isa::{self, Format, Inst, Op, csr, encode, reg_by_name};

/// An assembled program: memory contents from address 0 as little-endian
/// words, and for each word the 1-based source line it came from (0 for
/// padding).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Program {
    pub words: Vec<u32>,
    pub lines: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmError {
    /// 1-based source line.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for AsmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

type Res<T> = Result<T, String>;

/// Assembles `source`.
pub fn assemble(source: &str) -> Result<Program, AsmError> {
    let lines = source
        .lines()
        .enumerate()
        .map(|(i, text)| parse_line(text).map_err(|message| AsmError { line: i + 1, message }))
        .collect::<Result<Vec<Line>, _>>()?;
    let lines = local_labels(lines);

    // Pass 1: addresses of labels and the size of every statement.
    let mut symbols: HashMap<String, i64> = HashMap::new();
    let mut sizes: Vec<u32> = Vec::with_capacity(lines.len());
    let mut addr: u32 = 0;
    for (i, line) in lines.iter().enumerate() {
        let err = |message: String| AsmError { line: i + 1, message };
        for label in &line.labels {
            if symbols.insert(label.clone(), addr as i64).is_some() {
                return Err(err(format!("label '{label}' defined twice")));
            }
        }
        let size = match &line.stmt {
            Some(stmt) => {
                let ctx = Ctx { symbols: &symbols, pc: addr, final_pass: false };
                if let Stmt::Directive(name, args) = stmt
                    && (name == "equ" || name == "set")
                {
                    let (sym, value) = equ(args, &ctx).map_err(err)?;
                    symbols.insert(sym, value.ok_or_else(|| err("value must be known here".into()))?);
                    0
                } else {
                    size_of(stmt, &ctx).map_err(err)?
                }
            }
            None => 0,
        };
        sizes.push(size);
        addr = addr.checked_add(size).ok_or_else(|| err("program too large".into()))?;
    }

    // Pass 2: emit.
    let mut out = Out::default();
    for (i, line) in lines.iter().enumerate() {
        let Some(stmt) = &line.stmt else { continue };
        let err = |message: String| AsmError { line: i + 1, message };
        let start = out.bytes.len();
        let ctx = Ctx { symbols: &symbols, pc: start as u32, final_pass: true };
        out.line = i + 1;
        emit(stmt, sizes[i], &ctx, &mut out).map_err(err)?;
        if out.bytes.len() - start != sizes[i] as usize {
            return Err(err("internal error: size changed between passes".into()));
        }
    }
    Ok(out.finish())
}

#[derive(Debug, Default)]
struct Out {
    bytes: Vec<u8>,
    byte_lines: Vec<usize>,
    line: usize,
}

impl Out {
    fn byte(&mut self, b: u8) {
        self.bytes.push(b);
        self.byte_lines.push(self.line);
    }

    fn word(&mut self, w: u32) {
        for b in w.to_le_bytes() {
            self.byte(b);
        }
    }

    fn finish(self) -> Program {
        let words = self.bytes.chunks(4).map(|c| c.iter().rev().fold(0u32, |w, &b| w << 8 | b as u32)).collect();
        let lines =
            self.byte_lines.chunks(4).map(|c| c.iter().copied().find(|&l| l != 0).unwrap_or(0)).collect();
        Program { words, lines }
    }
}

// ------------------------------------------------------------------ parsing

#[derive(Debug)]
struct Line {
    labels: Vec<String>,
    stmt: Option<Stmt>,
}

#[derive(Debug)]
enum Stmt {
    /// `.name args`
    Directive(String, Vec<String>),
    /// `mnemonic operands`
    Inst(String, Vec<String>),
}

/// GNU local labels: `1:` may be defined many times; `1b` refers to the
/// last definition up to this line, `1f` to the next one after it. Each
/// definition gets a unique name (`.L1_0`, `.L1_1`, ...).
fn local_labels(mut lines: Vec<Line>) -> Vec<Line> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for line in &mut lines {
        for label in &mut line.labels {
            if label.bytes().all(|c| c.is_ascii_digit()) {
                let n = count.entry(label.clone()).or_insert(0);
                *label = format!(".L{label}_{n}");
                *n += 1;
            }
        }
        let args = match &mut line.stmt {
            Some(Stmt::Directive(d, _)) if matches!(d.as_str(), "ascii" | "asciz" | "string") => None,
            Some(Stmt::Inst(_, args) | Stmt::Directive(_, args)) => Some(args),
            None => None,
        };
        if let Some(args) = args {
            for arg in args {
                *arg = rewrite_local_refs(arg, &count);
            }
        }
    }
    lines
}

/// Replaces `Nb`/`Nf` tokens in an operand by the unique label names.
fn rewrite_local_refs(arg: &str, count: &HashMap<String, usize>) -> String {
    let b = arg.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let boundary = i == 0 || !is_ident(b[i - 1] as char);
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let dir = b.get(i).copied();
        let after = b.get(i + 1).is_none_or(|&c| !is_ident(c as char));
        if boundary && i > start && matches!(dir, Some(b'f' | b'b')) && after {
            let n = &arg[start..i];
            let defined = count.get(n).copied().unwrap_or(0);
            let k = if dir == Some(b'b') { defined.wrapping_sub(1) } else { defined };
            out += &format!(".L{n}_{k}");
            i += 1;
        } else if i > start {
            out += &arg[start..i];
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

fn strip_comment(text: &str) -> &str {
    let mut in_str = false;
    let mut in_chr = false;
    let b = text.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'"' if !in_chr && (i == 0 || b[i - 1] != b'\\') => in_str = !in_str,
            b'\'' if !in_str && (i == 0 || b[i - 1] != b'\\') => in_chr = !in_chr,
            b'#' | b';' if !in_str && !in_chr => return &text[..i],
            b'/' if !in_str && !in_chr && b.get(i + 1) == Some(&b'/') => return &text[..i],
            _ => {}
        }
    }
    text
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '.' || c == '$'
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$'
}

fn parse_line(text: &str) -> Res<Line> {
    let mut rest = strip_comment(text).trim();
    let mut labels = Vec::new();
    // Leading `name:` labels.
    loop {
        let end = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
        let numeric = end > 0 && rest[..end].bytes().all(|c| c.is_ascii_digit());
        if end > 0 && rest[end..].starts_with(':') && (numeric || rest.starts_with(is_ident_start)) {
            labels.push(rest[..end].to_string());
            rest = rest[end + 1..].trim_start();
        } else {
            break;
        }
    }
    if rest.is_empty() {
        return Ok(Line { labels, stmt: None });
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (head, args) = (&rest[..end], rest[end..].trim());
    let args = split_args(args);
    let stmt = match head.strip_prefix('.') {
        Some(d) => Stmt::Directive(d.to_ascii_lowercase(), args),
        None => Stmt::Inst(head.to_ascii_lowercase(), args),
    };
    Ok(Line { labels, stmt: Some(stmt) })
}

/// Splits at top-level commas (not inside parentheses, strings or chars).
fn split_args(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q && prev != '\\' {
                    quote = None;
                }
                cur.push(c);
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '(' => {
                    depth += 1;
                    cur.push(c);
                }
                ')' => {
                    depth -= 1;
                    cur.push(c);
                }
                ',' if depth == 0 => out.push(std::mem::take(&mut cur).trim().to_string()),
                _ => cur.push(c),
            },
        }
        prev = c;
    }
    out.push(cur.trim().to_string());
    out
}

// -------------------------------------------------------------- expressions

struct Ctx<'a> {
    symbols: &'a HashMap<String, i64>,
    pc: u32,
    /// In pass 2 every symbol must be known.
    final_pass: bool,
}

impl Ctx<'_> {
    /// Value of `expr`; `None` in pass 1 if it uses a symbol not yet defined.
    fn eval(&self, expr: &str) -> Res<Option<i64>> {
        let mut p = Parser { s: expr.trim().as_bytes(), i: 0, ctx: self, unknown: false };
        let v = p.expr()?;
        p.skip_ws();
        if p.i != p.s.len() {
            return Err(format!("cannot parse expression '{expr}'"));
        }
        Ok((!p.unknown).then_some(v))
    }

    /// Value of `expr`, which must be known (pass 2, or no forward reference).
    fn value(&self, expr: &str) -> Res<i64> {
        match self.eval(expr)? {
            Some(v) => Ok(v),
            None if !self.final_pass => Ok(0),
            None => Err(format!("unknown symbol in '{expr}'")),
        }
    }
}

struct Parser<'a, 'b> {
    s: &'a [u8],
    i: usize,
    ctx: &'b Ctx<'b>,
    unknown: bool,
}

impl Parser<'_, '_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s.get(self.i).copied()
    }

    fn expr(&mut self) -> Res<i64> {
        let mut v = self.term()?;
        loop {
            match self.peek() {
                Some(b'+') => {
                    self.i += 1;
                    v = v.wrapping_add(self.term()?);
                }
                Some(b'-') => {
                    self.i += 1;
                    v = v.wrapping_sub(self.term()?);
                }
                Some(b'|') => {
                    self.i += 1;
                    v |= self.term()?;
                }
                _ => return Ok(v),
            }
        }
    }

    fn term(&mut self) -> Res<i64> {
        let mut v = self.unary()?;
        loop {
            match self.peek() {
                Some(b'*') => {
                    self.i += 1;
                    v = v.wrapping_mul(self.unary()?);
                }
                Some(b'&') => {
                    self.i += 1;
                    v &= self.unary()?;
                }
                Some(b'<') if self.s.get(self.i + 1) == Some(&b'<') => {
                    self.i += 2;
                    v = v.wrapping_shl(self.unary()? as u32);
                }
                Some(b'>') if self.s.get(self.i + 1) == Some(&b'>') => {
                    self.i += 2;
                    v = v.wrapping_shr(self.unary()? as u32);
                }
                _ => return Ok(v),
            }
        }
    }

    fn unary(&mut self) -> Res<i64> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(self.unary()?.wrapping_neg())
            }
            Some(b'~') => {
                self.i += 1;
                Ok(!self.unary()?)
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err("missing ')'".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(b'%') => {
                self.i += 1;
                let start = self.i;
                while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.s[start..self.i]).unwrap().to_string();
                if self.peek() != Some(b'(') {
                    return Err(format!("'%{name}' needs '('"));
                }
                let v = self.unary()?;
                match name.as_str() {
                    "hi" => Ok((v.wrapping_add(0x800) >> 12) & 0xFFFFF),
                    "lo" => Ok(((v as i32) << 20 >> 20) as i64),
                    _ => Err(format!("unknown operator '%{name}'")),
                }
            }
            Some(b'\'') => {
                let rest = &self.s[self.i + 1..];
                let (c, len) = match rest {
                    [b'\\', e, b'\'', ..] => (unescape(*e as char)? as i64, 3),
                    [c, b'\'', ..] => (*c as i64, 2),
                    _ => return Err("bad character literal".into()),
                };
                self.i += 1 + len;
                Ok(c)
            }
            Some(c) if c.is_ascii_digit() => {
                let start = self.i;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap().replace('_', "");
                let t = t.to_ascii_lowercase();
                let parsed = if let Some(h) = t.strip_prefix("0x") {
                    i64::from_str_radix(h, 16)
                } else if let Some(b) = t.strip_prefix("0b") {
                    i64::from_str_radix(b, 2)
                } else {
                    t.parse()
                };
                parsed.map_err(|_| format!("bad number '{t}'"))
            }
            Some(c) if is_ident_start(c as char) => {
                let start = self.i;
                while self.i < self.s.len() && is_ident(self.s[self.i] as char) {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                if name == "." {
                    return Ok(self.ctx.pc as i64);
                }
                match self.ctx.symbols.get(name) {
                    Some(&v) => Ok(v),
                    None if !self.ctx.final_pass => {
                        self.unknown = true;
                        Ok(0)
                    }
                    None => Err(format!("unknown symbol '{name}'")),
                }
            }
            _ => Err("expected a value".into()),
        }
    }
}

fn unescape(c: char) -> Res<u8> {
    Ok(match c {
        'n' => b'\n',
        't' => b'\t',
        'r' => b'\r',
        '0' => 0,
        '\\' => b'\\',
        '\'' => b'\'',
        '"' => b'"',
        _ => return Err(format!("unknown escape '\\{c}'")),
    })
}

fn string_literal(arg: &str) -> Res<Vec<u8>> {
    let inner = arg
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| format!("expected a string, got '{arg}'"))?;
    let mut out = Vec::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push(unescape(chars.next().ok_or("unfinished escape")?)?);
        } else {
            let mut buf = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    Ok(out)
}

fn equ(args: &[String], ctx: &Ctx) -> Res<(String, Option<i64>)> {
    let [name, value] = args else { return Err(".equ needs a name and a value".into()) };
    Ok((name.clone(), ctx.eval(value)?))
}

// ------------------------------------------------------------- statements

fn reg(arg: &str) -> Res<u8> {
    reg_by_name(arg.trim()).ok_or_else(|| format!("expected a register, got '{arg}'"))
}

fn csr_num(arg: &str, ctx: &Ctx) -> Res<i32> {
    match csr::by_name(arg.trim()) {
        Some(c) => Ok(c as i32),
        None => Ok(ctx.value(arg)? as i32),
    }
}

/// `imm(rs1)`, `(rs1)` or `imm` (with x0).
fn mem_operand(arg: &str, ctx: &Ctx) -> Res<(i32, u8)> {
    let arg = arg.trim();
    if let Some(open) = arg.rfind('(')
        && arg.ends_with(')')
        && let Some(r) = reg_by_name(&arg[open + 1..arg.len() - 1])
    {
        let off = arg[..open].trim();
        let imm = if off.is_empty() { 0 } else { ctx.value(off)? as i32 };
        return Ok((imm, r));
    }
    Ok((ctx.value(arg)? as i32, 0))
}

fn want(args: &[String], n: usize, m: &str) -> Res<()> {
    if args.len() == n { Ok(()) } else { Err(format!("'{m}' takes {n} operand(s), got {}", args.len())) }
}

/// Bytes a statement emits; must not depend on symbols defined later
/// except through a fixed choice (as for `li` with an unknown value).
fn size_of(stmt: &Stmt, ctx: &Ctx) -> Res<u32> {
    match stmt {
        Stmt::Directive(name, args) => directive_size(name, args, ctx),
        Stmt::Inst(m, args) => Ok(match m.as_str() {
            "la" => 8,
            "li" => {
                want(args, 2, m)?;
                match ctx.eval(&args[1])? {
                    Some(v) => li_sequence(0, v)?.len() as u32 * 4,
                    None => 8,
                }
            }
            _ => 4,
        }),
    }
}

fn directive_size(name: &str, args: &[String], ctx: &Ctx) -> Res<u32> {
    let pc = ctx.pc;
    let align = |a: u32| -> Res<u32> { Ok((a - pc % a) % a) };
    Ok(match name {
        "text" | "data" | "bss" | "section" | "globl" | "global" | "option" | "type" | "size" | "file" | "local"
        | "equ" | "set" => 0,
        "word" | "4byte" | "long" => 4 * args.len() as u32,
        "half" | "2byte" | "short" => 2 * args.len() as u32,
        "byte" => args.len() as u32,
        "ascii" => args.iter().map(|a| string_literal(a).map(|s| s.len() as u32)).sum::<Res<u32>>()?,
        "asciz" | "string" => args.iter().map(|a| string_literal(a).map(|s| s.len() as u32 + 1)).sum::<Res<u32>>()?,
        "space" | "zero" | "skip" => {
            let n = ctx.eval(args.first().ok_or(".space needs a size")?)?.ok_or("size must be known here")?;
            u32::try_from(n).map_err(|_| "bad size".to_string())?
        }
        "align" | "p2align" | "balign" => {
            let n = ctx.eval(args.first().ok_or(".align needs a value")?)?.ok_or("alignment must be known here")?;
            let a = if name == "balign" { n } else { 1i64.checked_shl(n as u32).unwrap_or(0) };
            if a <= 0 || a & (a - 1) != 0 || a > 1 << 16 {
                return Err(format!("bad alignment {n}"));
            }
            align(a as u32)?
        }
        "org" => {
            let a = ctx.eval(args.first().ok_or(".org needs an address")?)?.ok_or("address must be known here")?;
            let a = u32::try_from(a).map_err(|_| "bad address".to_string())?;
            if a < pc {
                return Err(format!(".org 0x{a:x} is before the current address 0x{pc:x}"));
            }
            a - pc
        }
        _ => return Err(format!("unknown directive '.{name}'")),
    })
}

fn emit(stmt: &Stmt, size: u32, ctx: &Ctx, out: &mut Out) -> Res<()> {
    match stmt {
        Stmt::Directive(name, args) => match name.as_str() {
            "word" | "4byte" | "long" => {
                for a in args {
                    out.word(ctx.value(a)? as u32);
                }
            }
            "half" | "2byte" | "short" => {
                for a in args {
                    let v = ctx.value(a)? as u16;
                    out.byte(v as u8);
                    out.byte((v >> 8) as u8);
                }
            }
            "byte" => {
                for a in args {
                    out.byte(ctx.value(a)? as u8);
                }
            }
            "ascii" | "asciz" | "string" => {
                for a in args {
                    for b in string_literal(a)? {
                        out.byte(b);
                    }
                    if name != "ascii" {
                        out.byte(0);
                    }
                }
            }
            _ => {
                for _ in 0..size {
                    out.byte(0);
                }
            }
        },
        Stmt::Inst(m, args) => {
            for inst in expand(m, args, size, ctx)? {
                out.word(encode(&inst).map_err(|e| e.0)?);
            }
        }
    }
    Ok(())
}

/// `li rd, v` as instructions: `addi`, `lui`, or `lui` + `addi`.
fn li_sequence(rd: u8, v: i64) -> Res<Vec<Inst>> {
    if !(-(1i64 << 31)..(1i64 << 32)).contains(&v) {
        return Err(format!("value {v} does not fit in 32 bits"));
    }
    let v = v as u32 as i32;
    let lo = v << 20 >> 20;
    let hi = v.wrapping_sub(lo);
    Ok(if hi == 0 {
        vec![Inst { op: Op::Addi, rd, rs1: 0, rs2: 0, imm: lo }]
    } else if lo == 0 {
        vec![Inst { op: Op::Lui, rd, rs1: 0, rs2: 0, imm: hi }]
    } else {
        vec![Inst { op: Op::Lui, rd, rs1: 0, rs2: 0, imm: hi }, Inst { op: Op::Addi, rd, rs1: rd, rs2: 0, imm: lo }]
    })
}

fn i(op: Op, rd: u8, rs1: u8, rs2: u8, imm: i32) -> Inst {
    Inst { op, rd, rs1, rs2, imm }
}

/// The instructions of one source statement (`size` bytes, fixed in pass 1).
fn expand(m: &str, a: &[String], size: u32, ctx: &Ctx) -> Res<Vec<Inst>> {
    let rel = |arg: &str| -> Res<i32> { Ok((ctx.value(arg)? as u32).wrapping_sub(ctx.pc) as i32) };
    let one = |inst: Inst| Ok(vec![inst]);
    use Op::*;
    match m {
        "nop" => one(i(Addi, 0, 0, 0, 0)),
        "li" => {
            want(a, 2, m)?;
            let rd = reg(&a[0])?;
            let mut seq = li_sequence(rd, ctx.value(&a[1])?)?;
            if seq.len() * 4 < size as usize {
                // Fixed to two instructions in pass 1 (forward reference).
                let v = ctx.value(&a[1])? as u32 as i32;
                let lo = v << 20 >> 20;
                seq = vec![i(Lui, rd, 0, 0, v.wrapping_sub(lo)), i(Addi, rd, rd, 0, lo)];
            }
            Ok(seq)
        }
        "la" => {
            want(a, 2, m)?;
            let rd = reg(&a[0])?;
            let v = ctx.value(&a[1])? as u32 as i32;
            let lo = v << 20 >> 20;
            Ok(vec![i(Lui, rd, 0, 0, v.wrapping_sub(lo)), i(Addi, rd, rd, 0, lo)])
        }
        "mv" => {
            want(a, 2, m)?;
            one(i(Addi, reg(&a[0])?, reg(&a[1])?, 0, 0))
        }
        "not" => {
            want(a, 2, m)?;
            one(i(Xori, reg(&a[0])?, reg(&a[1])?, 0, -1))
        }
        "neg" => {
            want(a, 2, m)?;
            one(i(Sub, reg(&a[0])?, 0, reg(&a[1])?, 0))
        }
        "seqz" => {
            want(a, 2, m)?;
            one(i(Sltiu, reg(&a[0])?, reg(&a[1])?, 0, 1))
        }
        "snez" => {
            want(a, 2, m)?;
            one(i(Sltu, reg(&a[0])?, 0, reg(&a[1])?, 0))
        }
        "sltz" => {
            want(a, 2, m)?;
            one(i(Slt, reg(&a[0])?, reg(&a[1])?, 0, 0))
        }
        "sgtz" => {
            want(a, 2, m)?;
            one(i(Slt, reg(&a[0])?, 0, reg(&a[1])?, 0))
        }
        "beqz" | "bnez" | "bltz" | "bgez" | "blez" | "bgtz" => {
            want(a, 2, m)?;
            let r = reg(&a[0])?;
            let off = rel(&a[1])?;
            one(match m {
                "beqz" => i(Beq, 0, r, 0, off),
                "bnez" => i(Bne, 0, r, 0, off),
                "bltz" => i(Blt, 0, r, 0, off),
                "bgez" => i(Bge, 0, r, 0, off),
                "blez" => i(Bge, 0, 0, r, off),
                _ => i(Blt, 0, 0, r, off),
            })
        }
        "bgt" | "ble" | "bgtu" | "bleu" => {
            want(a, 3, m)?;
            let (r1, r2, off) = (reg(&a[0])?, reg(&a[1])?, rel(&a[2])?);
            let op = match m {
                "bgt" => Blt,
                "ble" => Bge,
                "bgtu" => Bltu,
                _ => Bgeu,
            };
            one(i(op, 0, r2, r1, off))
        }
        "j" => {
            want(a, 1, m)?;
            one(i(Jal, 0, 0, 0, rel(&a[0])?))
        }
        "jal" if a.len() == 1 => one(i(Jal, 1, 0, 0, rel(&a[0])?)),
        "call" => {
            want(a, 1, m)?;
            one(i(Jal, 1, 0, 0, rel(&a[0])?))
        }
        "tail" => {
            want(a, 1, m)?;
            one(i(Jal, 0, 0, 0, rel(&a[0])?))
        }
        "jr" => {
            want(a, 1, m)?;
            one(i(Jalr, 0, reg(&a[0])?, 0, 0))
        }
        "jalr" if a.len() == 1 => one(i(Jalr, 1, reg(&a[0])?, 0, 0)),
        "ret" => {
            want(a, 0, m)?;
            one(i(Jalr, 0, 1, 0, 0))
        }
        "csrr" => {
            want(a, 2, m)?;
            one(i(Csrrs, reg(&a[0])?, 0, 0, csr_num(&a[1], ctx)?))
        }
        "csrw" | "csrs" | "csrc" => {
            want(a, 2, m)?;
            let op = match m {
                "csrw" => Csrrw,
                "csrs" => Csrrs,
                _ => Csrrc,
            };
            one(i(op, 0, reg(&a[1])?, 0, csr_num(&a[0], ctx)?))
        }
        "csrwi" | "csrsi" | "csrci" => {
            want(a, 2, m)?;
            let op = match m {
                "csrwi" => Csrrwi,
                "csrsi" => Csrrsi,
                _ => Csrrci,
            };
            one(i(op, 0, uimm5(&a[1], ctx)?, 0, csr_num(&a[0], ctx)?))
        }
        _ => {
            let op = Op::by_mnemonic(m).ok_or_else(|| format!("unknown instruction '{m}'"))?;
            one(real(op, m, a, ctx)?)
        }
    }
}

fn uimm5(arg: &str, ctx: &Ctx) -> Res<u8> {
    let v = ctx.value(arg)?;
    if (0..32).contains(&v) { Ok(v as u8) } else { Err(format!("immediate {v} out of range (0..31)")) }
}

/// A real (non-pseudo) instruction.
fn real(op: Op, m: &str, a: &[String], ctx: &Ctx) -> Res<Inst> {
    let rel = |arg: &str| -> Res<i32> { Ok((ctx.value(arg)? as u32).wrapping_sub(ctx.pc) as i32) };
    let val = |arg: &str| -> Res<i32> { Ok(ctx.value(arg)? as i32) };
    Ok(match op.info().format {
        Format::R => {
            want(a, 3, m)?;
            i(op, reg(&a[0])?, reg(&a[1])?, reg(&a[2])?, 0)
        }
        Format::I | Format::Shift => {
            want(a, 3, m)?;
            i(op, reg(&a[0])?, reg(&a[1])?, 0, val(&a[2])?)
        }
        Format::Load | Format::Jalr => {
            if a.len() == 3 {
                i(op, reg(&a[0])?, reg(&a[1])?, 0, val(&a[2])?)
            } else {
                want(a, 2, m)?;
                let (imm, rs1) = mem_operand(&a[1], ctx)?;
                i(op, reg(&a[0])?, rs1, 0, imm)
            }
        }
        Format::S => {
            want(a, 2, m)?;
            let (imm, rs1) = mem_operand(&a[1], ctx)?;
            i(op, 0, rs1, reg(&a[0])?, imm)
        }
        Format::B => {
            want(a, 3, m)?;
            i(op, 0, reg(&a[0])?, reg(&a[1])?, rel(&a[2])?)
        }
        Format::U => {
            want(a, 2, m)?;
            let v = val(&a[1])?;
            if !(0..1 << 20).contains(&v) {
                return Err(format!("immediate {v} out of range (0..0xFFFFF)"));
            }
            i(op, reg(&a[0])?, 0, 0, v << 12)
        }
        Format::J => {
            if a.len() == 1 {
                i(op, 1, 0, 0, rel(&a[0])?)
            } else {
                want(a, 2, m)?;
                i(op, reg(&a[0])?, 0, 0, rel(&a[1])?)
            }
        }
        Format::Csr => {
            want(a, 3, m)?;
            i(op, reg(&a[0])?, reg(&a[2])?, 0, csr_num(&a[1], ctx)?)
        }
        Format::CsrI => {
            want(a, 3, m)?;
            i(op, reg(&a[0])?, uimm5(&a[2], ctx)?, 0, csr_num(&a[1], ctx)?)
        }
        Format::Fixed => {
            want(a, 0, m)?;
            Inst::new(op)
        }
        Format::Fence => Inst::new(op),
    })
}

/// One line per word: address, word and disassembly (for the viewer).
pub fn listing(p: &Program) -> Vec<String> {
    p.words.iter().enumerate().map(|(n, &w)| isa::disassemble(n as u32 * 4, w)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(src: &str) -> Vec<u32> {
        assemble(src).unwrap_or_else(|e| panic!("{e}")).words
    }

    #[test]
    fn assembles_instructions_and_labels() {
        let p = assemble("start: addi a0, zero, 5\nloop:\n  addi a0, a0, -1 # count\n  bnez a0, loop\n  ebreak").unwrap();
        assert_eq!(p.words, [0x0050_0513, 0xFFF5_0513, 0xFE05_1EE3, 0x0010_0073]);
        assert_eq!(p.lines, [1, 3, 4, 5]);
    }

    #[test]
    fn local_labels() {
        let p = words("1: j 1f\n1: j 1b\nj 1b\n2: beqz a0, 2b\nli a1, 0x1f\n.asciz \"1b\"");
        let q = words("a: j b\nb: j b\nj b\nc: beqz a0, c\nli a1, 0x1f\n.asciz \"1b\"");
        assert_eq!(p, q);
    }

    #[test]
    fn pseudo_instructions() {
        assert_eq!(words("li a0, 0x12345678"), [0x1234_5537, 0x6785_0513]);
        assert_eq!(words("li a0, -1"), [0xFFF0_0513]);
        assert_eq!(words("li t0, 0x10000000"), [0x1000_02B7]);
        assert_eq!(words("li a0, 0x800"), [0x0000_1537, 0x8005_0513]);
        // Forward reference: always two instructions.
        assert_eq!(words("li a0, X\n.equ X, 3"), [0x0000_0537, 0x0035_0513]);
        assert_eq!(words("ret\nmv a1, a0\nnop"), [0x0000_8067, 0x0005_0593, 0x0000_0013]);
        assert_eq!(words("csrw mtvec, t0\ncsrr a0, mcause"), [0x3052_9073, 0x3420_2573]);
        assert_eq!(words("call f\nf: ret"), [0x0040_00EF, 0x0000_8067]);
        assert_eq!(words("lw a0, %lo(x)(a1)\n.org 0x40\nx: .word 7"), {
            let mut w = vec![0x0405_A503];
            w.resize(16, 0);
            w.push(7);
            w
        });
    }

    #[test]
    fn data_directives() {
        let p = assemble(".byte 1, 2\n.half 0x0403\n.asciz \"hi\"\n.align 2\n.word -1").unwrap();
        assert_eq!(p.words, [0x0403_0201, 0x0000_6968, 0xFFFF_FFFF]);
    }

    #[test]
    fn reports_errors_with_line() {
        let e = assemble("nop\naddi a0, a0, 5000").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(e.message.contains("out of range"), "{}", e.message);
        assert_eq!(assemble("j nowhere").unwrap_err().message, "unknown symbol 'nowhere'");
        assert_eq!(assemble("frob a0").unwrap_err().message, "unknown instruction 'frob'");
    }
}

/// The assembler against LLVM's (skipped if `clang` and `llvm-objcopy` are
/// not installed): every instruction of the table plus the pseudo-instructions.
#[cfg(test)]
mod llvm_tests {
    use super::*;
    use std::process::Command;

    const SOURCE: &str = "
start:
  lui a0, 0x12345
  auipc t0, 1
  jal ra, fwd
  jalr t1, 8(a2)
  beq a0, a1, start
  bne s0, s1, fwd
  blt t3, t4, start
  bge t5, t6, fwd
  bltu a2, a3, start
  bgeu a4, a5, fwd
  lb a0, -1(sp)
  lh a1, 2(sp)
  lw a2, 2044(sp)
  lbu a3, -2048(gp)
  lhu a4, 0(tp)
  sb a0, 3(s0)
  sh a1, -6(s1)
  sw a2, 100(s2)
  addi s3, s4, -100
  slti s5, s6, 7
  sltiu s7, s8, 2047
  xori s9, s10, -1
  ori s11, ra, 0x55
  andi sp, gp, 0x7F
  slli tp, t0, 31
  srli t1, t2, 1
  srai s0, s1, 17
fwd:
  add a0, a1, a2
  sub a3, a4, a5
  sll a6, a7, s2
  slt s3, s4, s5
  sltu s6, s7, s8
  xor s9, s10, s11
  srl t3, t4, t5
  sra t6, zero, ra
  or sp, gp, tp
  and t0, t1, t2
  mul a0, a1, a2
  mulh a3, a4, a5
  mulhsu a6, a7, s0
  mulhu s1, s2, s3
  div s4, s5, s6
  divu s7, s8, s9
  rem s10, s11, t3
  remu t4, t5, t6
  ecall
  ebreak
  mret
  wfi
  csrrw a0, mstatus, a1
  csrrs a2, mie, a3
  csrrc a4, mtvec, a5
  csrrwi a6, mscratch, 31
  csrrsi a7, mepc, 1
  csrrci s0, mcause, 0
  nop
  li a0, 0x12345678
  li a1, -2048
  li a2, 0x12000
  li a3, 0xFFFFF000
  mv a0, a1
  not a2, a3
  neg a4, a5
  seqz a6, a7
  snez s0, s1
  beqz a0, start
  bnez a1, fwd
  blez a2, start
  bgez a3, fwd
  bltz a4, start
  bgtz a5, fwd
  bgt a0, a1, start
  ble a2, a3, fwd
  bgtu a4, a5, start
  bleu a6, a7, fwd
  j start
  jr t0
  ret
  csrr a0, mcause
  csrw mtvec, t0
  csrs mie, t1
  csrc mstatus, t2
  csrwi mstatus, 8
";

    #[test]
    fn matches_llvm() {
        let dir = std::env::temp_dir().join(format!("rv-asm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (s, o, bin) = (dir.join("t.s"), dir.join("t.o"), dir.join("t.bin"));
        std::fs::write(&s, SOURCE).unwrap();
        let clang = Command::new("clang")
            .args(["--target=riscv32", "-march=rv32im_zicsr", "-mno-relax", "-c"])
            .arg(&s)
            .arg("-o")
            .arg(&o)
            .output();
        let Ok(clang) = clang else {
            eprintln!("clang not found, skipping");
            return;
        };
        assert!(clang.status.success(), "{}", String::from_utf8_lossy(&clang.stderr));
        let copy = Command::new("llvm-objcopy").args(["-O", "binary", "-j", ".text"]).arg(&o).arg(&bin).output();
        let Ok(copy) = copy else {
            eprintln!("llvm-objcopy not found, skipping");
            return;
        };
        assert!(copy.status.success());
        let bytes = std::fs::read(&bin).unwrap();
        let want: Vec<u32> = bytes.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let got = assemble(SOURCE).unwrap().words;
        for (n, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "word {n}: ours {} / llvm {}", isa::disassemble(n as u32 * 4, *g), isa::disassemble(n as u32 * 4, *w));
        }
        assert_eq!(got.len(), want.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
