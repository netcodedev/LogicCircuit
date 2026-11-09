//! Elaboration of CLDL sources: loops and width parameters.
//!
//! A `.cldl` file is first parsed into a `Template`, which still contains
//! `FOR` loops and expressions over parameters (`Data[W]`, `FA[i+1].Cin`,
//! `REGISTER<W>`). `Template::elaborate` evaluates them for concrete parameter
//! values and produces a plain `CldlDefinition` — the form the rest of the
//! builder (planning, flatten, views, export) has always worked on.
//!
//! Naming:
//! - a parametric definition elaborated with values gets the name
//!   `NAME<v1,v2>` (e.g. `REGISTER<16>`), see `instance_name`
//! - an indexed part `PART FA[i]` becomes the part `FA_<i>` (e.g. `FA_3`), and
//!   `FA[i].Cout` refers to its pin `FA_3.Cout`

use std::collections::HashMap;

use pest::{Parser, iterators::Pair};

use super::{
    CldlDefinition, CldlParser, ConnectionDef, InitCommand, InitValue, IoPinDef, PartDef, PinRef,
    PinValue, Rule, SeqStep,
};

/// 1-based source position (line, column) for error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    fn of(pair: &Pair<Rule>) -> Self {
        let (line, col) = pair.as_span().start_pos().line_col();
        Pos { line, col }
    }
}

#[derive(Debug, Clone)]
enum Expr {
    Num(i64),
    Var(String, Pos),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>, Pos),
}

/// A pin path with unevaluated indices: `[("FA", Some(i)), ("Cout", None)]`.
#[derive(Debug, Clone)]
struct PinTemplate {
    segments: Vec<(String, Option<Expr>)>,
}

#[derive(Debug, Clone)]
struct IoTemplate {
    name: String,
    width: Option<Expr>,
}

#[derive(Debug, Clone)]
enum InitTemplate {
    Set(Vec<(PinTemplate, InitValue)>),
    Eval,
    Seq(Vec<Vec<(PinTemplate, InitValue)>>),
}

#[derive(Debug, Clone)]
enum Item {
    Input(Vec<IoTemplate>),
    Output(Vec<IoTemplate>),
    Part {
        name: String,
        index: Option<Expr>,
        part_type: String,
        args: Vec<Expr>,
    },
    Connect(PinTemplate, PinTemplate),
    For {
        var: String,
        pos: Pos,
        start: Expr,
        end: Expr,
        inclusive: bool,
        body: Vec<Item>,
    },
    Init(Vec<InitTemplate>),
}

/// A parsed circuit definition before elaboration.
#[derive(Debug, Clone)]
pub struct Template {
    pub name: String,
    /// Parameter names, e.g. `["W"]` for `CIRCUIT REGISTER<W>`; empty for an
    /// ordinary definition.
    pub params: Vec<String>,
    param_pos: Vec<Pos>,
    items: Vec<Item>,
    /// See `CldlDefinition::category`.
    pub category: Option<String>,
}

/// The definition name for `name` elaborated with `args`: `REGISTER<16>`,
/// `MUX<16,4>`, or just `name` without parameters.
pub fn instance_name(name: &str, args: &[i64]) -> String {
    if args.is_empty() {
        name.to_string()
    } else {
        let args: Vec<String> = args.iter().map(i64::to_string).collect();
        format!("{name}<{}>", args.join(","))
    }
}

/// Splits `REGISTER<16>` (spaces allowed) into `("REGISTER", [16])`.
/// Returns `None` for names without a parameter list or malformed ones.
pub fn split_instance_name(name: &str) -> Option<(&str, Vec<i64>)> {
    let open = name.find('<')?;
    let inner = name[open + 1..].trim_end().strip_suffix('>')?;
    let args = inner
        .split(',')
        .map(|a| a.trim().parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?;
    Some((name[..open].trim(), args))
}

/// Parses CLDL source into a template. Syntax errors carry pest's line/column.
pub fn parse_template(source: &str) -> Result<Template, String> {
    let mut file = CldlParser::parse(Rule::file, source).map_err(|e| e.to_string())?;
    let circuit = file.next().unwrap().into_inner().next().unwrap();
    assert_eq!(circuit.as_rule(), Rule::circuit);

    let mut name = String::new();
    let mut params = Vec::new();
    let mut param_pos = Vec::new();
    let mut items = Vec::new();
    for pair in circuit.into_inner() {
        match pair.as_rule() {
            Rule::ident => name = pair.as_str().to_string(),
            Rule::params => {
                for p in pair.into_inner() {
                    let pos = Pos::of(&p);
                    let param = p.as_str().to_string();
                    if params.contains(&param) {
                        return Err(format!(
                            "{name}: line {}, column {}: parameter '{param}' declared twice",
                            pos.line, pos.col
                        ));
                    }
                    params.push(param);
                    param_pos.push(pos);
                }
            }
            Rule::circuit_body => {
                for item in pair.into_inner() {
                    items.push(parse_item(item));
                }
            }
            r => unreachable!("Unexpected rule in circuit: {r:?}"),
        }
    }
    Ok(Template { name, params, param_pos, items, category: None })
}

fn parse_item(pair: Pair<Rule>) -> Item {
    match pair.as_rule() {
        Rule::input_def => Item::Input(pair.into_inner().map(parse_io).collect()),
        Rule::output_def => Item::Output(pair.into_inner().map(parse_io).collect()),
        Rule::part_def => {
            let mut name = String::new();
            let mut index = None;
            let mut part_type = String::new();
            let mut args = Vec::new();
            for p in pair.into_inner() {
                match p.as_rule() {
                    Rule::ident => name = p.as_str().to_string(),
                    Rule::index => index = Some(parse_index(p)),
                    Rule::part_type => {
                        for t in p.into_inner() {
                            match t.as_rule() {
                                Rule::ident => part_type = t.as_str().to_string(),
                                Rule::type_args => args = t.into_inner().map(parse_expr).collect(),
                                r => unreachable!("Unexpected rule in part_type: {r:?}"),
                            }
                        }
                    }
                    r => unreachable!("Unexpected rule in part_def: {r:?}"),
                }
            }
            Item::Part { name, index, part_type, args }
        }
        Rule::connect_def => {
            let mut pins = pair.into_inner().map(parse_pin);
            Item::Connect(pins.next().unwrap(), pins.next().unwrap())
        }
        Rule::for_loop => {
            let mut inner = pair.into_inner();
            let var_pair = inner.next().unwrap();
            let pos = Pos::of(&var_pair);
            let var = var_pair.as_str().to_string();
            let start = parse_expr(inner.next().unwrap());
            let inclusive = inner.next().unwrap().as_str() == "..=";
            let end = parse_expr(inner.next().unwrap());
            let body = inner.map(parse_item).collect();
            Item::For { var, pos, start, end, inclusive, body }
        }
        Rule::init_block => Item::Init(
            pair.into_inner()
                .map(|cmd| {
                    let cmd = cmd.into_inner().next().unwrap();
                    match cmd.as_rule() {
                        Rule::set_command => InitTemplate::Set(cmd.into_inner().map(parse_pin_value).collect()),
                        Rule::eval_command => InitTemplate::Eval,
                        Rule::seq_command => InitTemplate::Seq(
                            cmd.into_inner()
                                .map(|step| step.into_inner().map(parse_pin_value).collect())
                                .collect(),
                        ),
                        r => unreachable!("Unexpected rule in init_command: {r:?}"),
                    }
                })
                .collect(),
        ),
        r => unreachable!("Unexpected rule in circuit body: {r:?}"),
    }
}

fn parse_io(pair: Pair<Rule>) -> IoTemplate {
    let mut inner = pair.into_inner();
    let name = inner.next().unwrap().as_str().to_string();
    let width = inner.next().map(parse_index);
    IoTemplate { name, width }
}

fn parse_index(pair: Pair<Rule>) -> Expr {
    assert_eq!(pair.as_rule(), Rule::index);
    parse_expr(pair.into_inner().next().unwrap())
}

fn parse_pin(pair: Pair<Rule>) -> PinTemplate {
    assert_eq!(pair.as_rule(), Rule::pin);
    let segments = pair
        .into_inner()
        .map(|seg| {
            let mut inner = seg.into_inner();
            let name = inner.next().unwrap().as_str().to_string();
            (name, inner.next().map(parse_index))
        })
        .collect();
    PinTemplate { segments }
}

fn parse_pin_value(pair: Pair<Rule>) -> (PinTemplate, InitValue) {
    let mut inner = pair.into_inner();
    let pin = parse_pin(inner.next().unwrap());
    let value = match inner.next().unwrap().as_str() {
        "0" => InitValue::Zero,
        _ => InitValue::One,
    };
    (pin, value)
}

fn parse_expr(pair: Pair<Rule>) -> Expr {
    match pair.as_rule() {
        Rule::expr => {
            let mut inner = pair.into_inner();
            let mut acc = parse_expr(inner.next().unwrap());
            while let Some(op) = inner.next() {
                let rhs = parse_expr(inner.next().unwrap());
                acc = match op.as_str() {
                    "+" => Expr::Add(Box::new(acc), Box::new(rhs)),
                    _ => Expr::Sub(Box::new(acc), Box::new(rhs)),
                };
            }
            acc
        }
        Rule::term => {
            let mut inner = pair.into_inner();
            let mut acc = parse_expr(inner.next().unwrap());
            while let Some(op) = inner.next() {
                let pos = Pos::of(&op);
                let rhs = parse_expr(inner.next().unwrap());
                acc = Expr::Mul(Box::new(acc), Box::new(rhs), pos);
            }
            acc
        }
        // Numbers too large for i64 are reported as out of range on use.
        Rule::number => Expr::Num(pair.as_str().parse().unwrap_or(i64::MAX)),
        Rule::ident => Expr::Var(pair.as_str().to_string(), Pos::of(&pair)),
        r => unreachable!("Unexpected rule in expression: {r:?}"),
    }
}

/// Values of parameters and loop variables while elaborating.
struct Scope<'a> {
    circuit: &'a str,
    vars: HashMap<String, i64>,
}

impl Scope<'_> {
    fn error(&self, pos: Pos, message: impl std::fmt::Display) -> String {
        format!("{}: line {}, column {}: {message}", self.circuit, pos.line, pos.col)
    }

    fn eval(&self, expr: &Expr) -> Result<i64, String> {
        let overflow = |pos: Pos| self.error(pos, "arithmetic overflow");
        Ok(match expr {
            Expr::Num(n) => *n,
            Expr::Var(name, pos) => *self
                .vars
                .get(name)
                .ok_or_else(|| self.error(*pos, format!("unknown parameter or loop variable '{name}'")))?,
            Expr::Add(a, b) => self.eval(a)?.checked_add(self.eval(b)?).ok_or_else(|| overflow(first_pos(a)))?,
            Expr::Sub(a, b) => self.eval(a)?.checked_sub(self.eval(b)?).ok_or_else(|| overflow(first_pos(a)))?,
            Expr::Mul(a, b, pos) => self.eval(a)?.checked_mul(self.eval(b)?).ok_or_else(|| overflow(*pos))?,
        })
    }

    /// Evaluates an index (bit or part index), which must be >= 0.
    fn index(&self, expr: &Expr) -> Result<usize, String> {
        let v = self.eval(expr)?;
        usize::try_from(v).map_err(|_| self.error(first_pos(expr), format!("index {v} is negative")))
    }

    /// Evaluates a bus width, which must be >= 1.
    fn width(&self, expr: &Expr) -> Result<usize, String> {
        let v = self.eval(expr)?;
        if v < 1 {
            return Err(self.error(first_pos(expr), format!("bus width {v} must be at least 1")));
        }
        Ok(v as usize)
    }

    fn pin(&self, pin: &PinTemplate) -> Result<PinRef, String> {
        let last = pin.segments.len() - 1;
        let mut names = Vec::with_capacity(pin.segments.len());
        let mut index = None;
        for (i, (name, idx)) in pin.segments.iter().enumerate() {
            match idx {
                None => names.push(name.clone()),
                Some(e) if i == last => {
                    names.push(name.clone());
                    index = Some(self.index(e)?);
                }
                Some(e) => names.push(indexed_name(name, self.index(e)?)),
            }
        }
        Ok(PinRef { base_name: names.join("."), index })
    }

    fn pin_values(&self, values: &[(PinTemplate, InitValue)]) -> Result<Vec<PinValue>, String> {
        values
            .iter()
            .map(|(pin, value)| Ok(PinValue { pin_ref: self.pin(pin)?, value: value.clone() }))
            .collect()
    }
}

/// Position of the leftmost variable in an expression, or line 0 for pure
/// numbers (only reachable for out-of-range literals).
fn first_pos(expr: &Expr) -> Pos {
    match expr {
        Expr::Num(_) => Pos { line: 0, col: 0 },
        Expr::Var(_, pos) => *pos,
        Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b, _) => match first_pos(a) {
            Pos { line: 0, .. } => first_pos(b),
            p => p,
        },
    }
}

/// Name of the part `name[index]`: `FA_3`.
pub fn indexed_name(name: &str, index: usize) -> String {
    format!("{name}_{index}")
}

impl Template {
    /// Evaluates loops and expressions for the given parameter values.
    pub fn elaborate(&self, args: &[i64]) -> Result<CldlDefinition, String> {
        let name = instance_name(&self.name, args);
        if args.len() != self.params.len() {
            return Err(format!(
                "{name}: '{}' expects {} parameter(s) <{}>, got {}",
                self.name,
                self.params.len(),
                self.params.join(", "),
                args.len()
            ));
        }
        let mut scope = Scope { circuit: &self.name, vars: HashMap::new() };
        for ((param, pos), &value) in self.params.iter().zip(&self.param_pos).zip(args) {
            if value < 1 {
                return Err(scope.error(*pos, format!("parameter {param} = {value} must be at least 1 (in {name})")));
            }
            scope.vars.insert(param.clone(), value);
        }

        let mut def = CldlDefinition {
            name,
            inputs: Vec::new(),
            outputs: Vec::new(),
            parts: Vec::new(),
            connections: Vec::new(),
            initialization: Vec::new(),
            category: self.category.clone(),
        };
        elaborate_items(&self.items, &mut scope, &mut def)?;
        expand_bus_inits(&mut def);
        Ok(def)
    }
}

/// In INIT blocks, an assignment to one of the definition's own buses without
/// an index (`SET Data 0`) applies to every bit, so register templates can
/// clear `Data[W]` without a loop.
fn expand_bus_inits(def: &mut CldlDefinition) {
    let widths: HashMap<String, usize> = def
        .inputs
        .iter()
        .chain(&def.outputs)
        .filter_map(|p| Some((p.name.clone(), p.width?)))
        .collect();
    let expand = |values: &mut Vec<PinValue>| {
        *values = values
            .drain(..)
            .flat_map(|pv| match (pv.pin_ref.index, widths.get(&pv.pin_ref.base_name)) {
                (None, Some(&w)) => (0..w)
                    .map(|i| PinValue {
                        pin_ref: PinRef { base_name: pv.pin_ref.base_name.clone(), index: Some(i) },
                        value: pv.value.clone(),
                    })
                    .collect(),
                _ => vec![pv],
            })
            .collect();
    };
    for cmd in &mut def.initialization {
        match cmd {
            InitCommand::Set(values) => expand(values),
            InitCommand::Seq(steps) => steps.iter_mut().for_each(|s| expand(&mut s.assignments)),
            InitCommand::Eval => {}
        }
    }
}

fn elaborate_items(items: &[Item], scope: &mut Scope, def: &mut CldlDefinition) -> Result<(), String> {
    for item in items {
        match item {
            Item::Input(pins) | Item::Output(pins) => {
                let pins = pins
                    .iter()
                    .map(|p| {
                        Ok(IoPinDef {
                            name: p.name.clone(),
                            width: p.width.as_ref().map(|w| scope.width(w)).transpose()?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                if matches!(item, Item::Input(_)) {
                    def.inputs.extend(pins);
                } else {
                    def.outputs.extend(pins);
                }
            }
            Item::Part { name, index, part_type, args } => {
                let inst = match index {
                    Some(e) => indexed_name(name, scope.index(e)?),
                    None => name.clone(),
                };
                let args = args.iter().map(|a| scope.eval(a)).collect::<Result<Vec<_>, _>>()?;
                def.parts.push(PartDef { instance_name: inst, part_type: instance_name(part_type, &args) });
            }
            Item::Connect(src, dest) => def.connections.push(ConnectionDef {
                source_pin: scope.pin(src)?,
                dest_pin: scope.pin(dest)?,
            }),
            Item::For { var, pos, start, end, inclusive, body } => {
                if scope.vars.contains_key(var) {
                    return Err(scope.error(*pos, format!("loop variable '{var}' shadows a parameter or outer loop variable")));
                }
                let from = scope.eval(start)?;
                let to = scope.eval(end)?;
                let to = if *inclusive { to.saturating_add(1) } else { to };
                if to < from {
                    let op = if *inclusive { "..=" } else { ".." };
                    return Err(scope.error(
                        *pos,
                        format!("range {from}{op}{} of '{var}' runs backwards", scope.eval(end)?),
                    ));
                }
                for i in from..to {
                    scope.vars.insert(var.clone(), i);
                    let result = elaborate_items(body, scope, def);
                    scope.vars.remove(var);
                    result?;
                }
            }
            Item::Init(commands) => {
                for cmd in commands {
                    def.initialization.push(match cmd {
                        InitTemplate::Set(values) => InitCommand::Set(scope.pin_values(values)?),
                        InitTemplate::Eval => InitCommand::Eval,
                        InitTemplate::Seq(steps) => InitCommand::Seq(
                            steps
                                .iter()
                                .map(|s| Ok(SeqStep { assignments: scope.pin_values(s)? }))
                                .collect::<Result<_, String>>()?,
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cldl::CircuitBuilder;

    fn elaborate(src: &str, args: &[i64]) -> Result<CldlDefinition, String> {
        parse_template(src)?.elaborate(args)
    }

    fn connections(def: &CldlDefinition) -> Vec<String> {
        def.connections
            .iter()
            .map(|c| format!("{} -> {}", c.source_pin.to_wire_map_key(), c.dest_pin.to_wire_map_key()))
            .collect()
    }

    #[test]
    fn unrolls_loops_with_indexed_parts() {
        let def = elaborate(
            "CIRCUIT CHAIN<W> {
                INPUT A[W]; OUTPUT Y;
                FOR i IN 0..W { PART G[i]: AND; CONNECT A[i] -> G[i].in1; }
                FOR i IN 1..W { CONNECT G[i-1].out -> G[i].in2; }
                CONNECT G[W-1].out -> Y;
            }",
            &[3],
        )
        .unwrap();
        assert_eq!(def.name, "CHAIN<3>");
        assert_eq!(def.inputs[0].width, Some(3));
        let parts: Vec<_> = def.parts.iter().map(|p| p.instance_name.as_str()).collect();
        assert_eq!(parts, ["G_0", "G_1", "G_2"]);
        assert_eq!(
            connections(&def),
            [
                "A.0 -> G_0.in1",
                "A.1 -> G_1.in1",
                "A.2 -> G_2.in1",
                "G_0.out -> G_1.in2",
                "G_1.out -> G_2.in2",
                "G_2.out -> Y",
            ]
        );
    }

    #[test]
    fn nested_loops_inclusive_ranges_and_arithmetic() {
        let def = elaborate(
            "CIRCUIT GRID<N> {
                INPUT X[2*N+1];
                FOR r IN 0..=N-1 { FOR c IN 0..N {
                    PART C[r*N+c]: NOT;
                    CONNECT X[(r+c)*2] -> C[r*N+c].in;
                } }
            }",
            &[2],
        )
        .unwrap();
        assert_eq!(def.inputs[0].width, Some(5));
        let parts: Vec<_> = def.parts.iter().map(|p| p.instance_name.as_str()).collect();
        assert_eq!(parts, ["C_0", "C_1", "C_2", "C_3"]);
        assert_eq!(
            connections(&def),
            ["X.0 -> C_0.in", "X.2 -> C_1.in", "X.2 -> C_2.in", "X.4 -> C_3.in"]
        );
    }

    #[test]
    fn parameter_values_in_part_types_and_bus_inits() {
        let def = elaborate(
            "CIRCUIT TOP<W> {
                INPUT D[W];
                PART R: REGISTER<W*2>; PART M: MUX<W, 4>; PART N: NAND;
                INIT { SET D 1; SEQ D[W-1] 0 -> D 0; }
            }",
            &[8],
        )
        .unwrap();
        let types: Vec<_> = def.parts.iter().map(|p| p.part_type.as_str()).collect();
        assert_eq!(types, ["REGISTER<16>", "MUX<8,4>", "NAND"]);
        let InitCommand::Set(values) = &def.initialization[0] else { panic!() };
        assert_eq!(values.len(), 8);
        let InitCommand::Seq(steps) = &def.initialization[1] else { panic!() };
        assert_eq!(steps[0].assignments.len(), 1);
        assert_eq!(steps[1].assignments.len(), 8);
    }

    #[test]
    fn plain_definitions_are_unchanged() {
        let def = elaborate("CIRCUIT X { INPUT a, B[4]; OUTPUT y; PART g: NAND; CONNECT a -> g.in1; CONNECT B[3] -> g.in2; }", &[]).unwrap();
        assert_eq!(def.name, "X");
        assert_eq!(connections(&def), ["a -> g.in1", "B.3 -> g.in2"]);
    }

    #[test]
    fn instance_names() {
        assert_eq!(instance_name("REGISTER", &[16]), "REGISTER<16>");
        assert_eq!(instance_name("MUX", &[16, 4]), "MUX<16,4>");
        assert_eq!(split_instance_name("MUX< 16 , 4 >"), Some(("MUX", vec![16, 4])));
        assert_eq!(split_instance_name("ADDER_8B"), None);
        assert_eq!(split_instance_name("ADDER<W>"), None);
    }

    #[test]
    fn errors_name_line_and_column() {
        let err = |src: &str, args: &[i64]| elaborate(src, args).unwrap_err();
        assert_eq!(
            err("CIRCUIT A<W> {\n  INPUT X[N];\n}", &[4]),
            "A: line 2, column 11: unknown parameter or loop variable 'N'"
        );
        assert_eq!(
            err("CIRCUIT A<W> {\n  FOR i IN W..0 { }\n}", &[4]),
            "A: line 2, column 7: range 4..0 of 'i' runs backwards"
        );
        assert_eq!(
            err("CIRCUIT A<W> {\n  FOR W IN 0..2 { }\n}", &[4]),
            "A: line 2, column 7: loop variable 'W' shadows a parameter or outer loop variable"
        );
        assert_eq!(
            err("CIRCUIT A<W> {\n  INPUT X[W-4];\n}", &[4]),
            "A: line 2, column 11: bus width 0 must be at least 1"
        );
        assert_eq!(
            err("CIRCUIT A<W> {\n  INPUT X[W];\n  CONNECT X[W-5] -> Y;\n}", &[4]),
            "A: line 3, column 13: index -1 is negative"
        );
        assert_eq!(err("CIRCUIT A<W> { }", &[]), "A: 'A' expects 1 parameter(s) <W>, got 0");
        assert_eq!(err("CIRCUIT A<W> { }", &[4, 2]), "A<4,2>: 'A' expects 1 parameter(s) <W>, got 2");
        assert_eq!(err("CIRCUIT A<W> { }", &[0]), "A: line 1, column 11: parameter W = 0 must be at least 1 (in A<0>)");
        assert_eq!(err("CIRCUIT A<W, W> { }", &[]), "A: line 1, column 14: parameter 'W' declared twice");
        // Syntax errors come from pest, with line and column.
        assert!(err("CIRCUIT A<W> {\n  FOR i IN 0..W PART X: NOT;\n}", &[1]).contains("2:17"));
    }

    #[test]
    fn empty_ranges_are_allowed() {
        let def = elaborate("CIRCUIT A<W> { FOR i IN 1..W { PART P[i]: NOT; } FOR i IN 0..=W-2 { PART Q[i]: NOT; } }", &[1]).unwrap();
        assert!(def.parts.is_empty());
    }

    fn builder() -> CircuitBuilder {
        CircuitBuilder::new()
    }

    #[test]
    fn instantiates_templates_on_demand() {
        let mut b = builder();
        b.load_cldl_str("CIRCUIT INV_BUS<W> { INPUT A[W]; OUTPUT Y[W]; FOR i IN 0..W { PART N[i]: NOT; CONNECT A[i] -> N[i].in; CONNECT N[i].out -> Y[i]; } }")
            .unwrap();
        b.load_cldl_str("CIRCUIT USES_INV { INPUT A[3]; OUTPUT Y[3]; PART I: INV_BUS<3>; CONNECT A -> I.A; CONNECT I.Y -> Y; }")
            .unwrap();
        b.check("USES_INV").unwrap();
        assert!(b.circuit_names().contains(&"INV_BUS<3>".to_string()));
        assert_eq!(b.nand_count("USES_INV"), Some(3));
        assert_eq!(b.nand_count("INV_BUS< 5 >"), Some(5));
        let c = b.build("USES_INV");
        for (i, v) in [true, false, true].into_iter().enumerate() {
            c.set_input(&format!("A.{i}"), v);
        }
        c.evaluate_stable();
        let y: Vec<_> = (0..3).map(|i| c.get_output(&format!("Y.{i}")).unwrap()).collect();
        assert_eq!(y, [false, true, false]);
        // Errors inside a template surface when it is instantiated.
        b.load_cldl_str("CIRCUIT BAD<W> { INPUT A[W-2]; }").unwrap();
        b.load_cldl_str("CIRCUIT USES_BAD { PART B: BAD<2>; }").unwrap();
        assert_eq!(b.check("USES_BAD").unwrap_err(), "BAD: line 1, column 26: bus width 0 must be at least 1");
        assert_eq!(b.check("USES_NOTHING").unwrap_err(), "Unknown circuit 'USES_NOTHING'");
    }

    /// Reads bus `name` (`name.0` = LSB) of a built circuit.
    fn read(c: &crate::circuit::Circuit, name: &str, w: usize) -> u32 {
        (0..w).map(|i| (c.get_output(&format!("{name}.{i}")).unwrap() as u32) << i).sum()
    }

    #[test]
    fn parametric_adder_matches_fixed_adder() {
        let b = builder();
        assert_eq!(b.nand_count("ADDER<8>"), b.nand_count("ADDER_8B"));
        assert_eq!(b.nand_count("ADDER<4>"), b.nand_count("ADDER_4B"));
        assert_eq!(b.nand_count("REGISTER<8>"), b.nand_count("REGISTER_8B"));
        assert_eq!(b.nand_count("REGISTER<4>"), b.nand_count("REGISTER_4B"));
        assert_eq!(b.nand_count("MUX_2_1_BUS<8>"), b.nand_count("MUX_8B_2_1"));
        assert_eq!(b.nand_count("ADDER<16>"), Some(2 * b.nand_count("ADDER_8B").unwrap()));
        for (param, fixed) in [
            ("MUX_4_1_BUS<8>", "MUX_8B_4_1"),
            ("MUX_16_1_BUS<8>", "MUX_8B_16_1"),
            ("REG_FILE_16<8>", "REG_FILE_16x8"),
            ("AND_BUS<8>", "AND_8B"),
            ("XOR_BUS<4>", "XOR_4B"),
        ] {
            assert_eq!(b.nand_count(param), b.nand_count(fixed), "{param}");
        }

        let (fixed, param) = (b.build("ADDER_8B"), b.build("ADDER<8>"));
        for x in (0..256u32).step_by(7) {
            for y in (0..256u32).step_by(5) {
                for cin in [false, true] {
                    for c in [&fixed, &param] {
                        for i in 0..8 {
                            c.set_input(&format!("A.{i}"), (x >> i) & 1 == 1);
                            c.set_input(&format!("B.{i}"), (y >> i) & 1 == 1);
                        }
                        c.set_input("Cin", cin);
                        c.evaluate_stable();
                    }
                    let sum = x + y + cin as u32;
                    assert_eq!(read(&param, "S", 8), sum & 0xFF);
                    assert_eq!(read(&fixed, "S", 8), read(&param, "S", 8));
                    assert_eq!(fixed.get_output("Cout"), param.get_output("Cout"));
                }
            }
        }
    }
}
