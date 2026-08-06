//! The document calculation engine.
//!
//! Piers's brief (2026-08-07), which is also the acceptance test in `tests`:
//!
//! > `$pay_rate * $$superannuation_rate = $super_amount`, `$pay_rate + $super_amount =
//! > $sub_total` … `SUM($hours)`, `SUM($pay_amount)` … `$` = float entered on screen (DP=4).
//! > `$$` = a referenced cell out of band, eg `$A$1 = "Super Rate"[text]`, `$B$1 = "11.5000"
//! > [float]. That cell is `$$superannuation_rate` in any calculation.
//!
//! So there are exactly three kinds of name, and that is the whole vocabulary:
//!
//! - **`$column`** — the value in that column, *on the row being calculated*. Columns are named
//!   by their heading, normalised (`"Pay Rate"` → `pay_rate`), so the document reads as the
//!   formula does.
//! - **`$$constant`** — a value from a label/value pair somewhere else in the document: the label
//!   cell names it, the cell beside it holds the number. Rates live in one visible place rather
//!   than being typed into thirty formulas.
//! - **`SUM($column)`** and friends — down a whole column, for a totals row.
//!
//! ## Why order does not need declaring
//!
//! Formulas name what they need, so the engine reads the dependencies out of them and evaluates in
//! that order (Kahn's algorithm). Piers: *"Most calculations are order based anyway."* They are —
//! but writing that order down by hand is how spreadsheets rot, so the engine derives it and
//! reports a genuine cycle as an error in the cell rather than looping.
//!
//! ## Errors are values
//!
//! A formula that divides by zero, names a column that does not exist, or sits in a cycle
//! produces [`Value::Error`], which renders as `#REF`/`#DIV0`/`#CYCLE` in the cell. It never
//! panics and never silently produces a zero — a wrong number that looks like a right number is
//! the worst thing a calculator can do.

use std::collections::{HashMap, HashSet};

/// Decimal places for entered and displayed floats (Piers: "DP=4").
pub const DP: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    Text(String),
    Error(&'static str),
    Empty,
}

impl Value {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            // A number typed into a cell arrives as text; treat it as the number it plainly is.
            Value::Text(t) => t.trim().replace(['$', ',', '%'], "").parse().ok(),
            _ => None,
        }
    }

    /// How the value is written into the document.
    pub fn display(&self) -> String {
        match self {
            Value::Number(n) => format!("{n:.*}", DP),
            Value::Text(t) => t.clone(),
            Value::Error(e) => (*e).to_string(),
            Value::Empty => String::new(),
        }
    }
}

/// Normalise a heading or label into the name a formula uses: lowercase, non-alphanumerics to
/// underscores, no leading/trailing/repeated underscores. `"Total Cost / Hour"` → `total_cost_hour`.
pub fn name_of(label: &str) -> String {
    let mut out = String::new();
    let mut last_us = true; // suppresses a leading underscore
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// One column of the sheet: its name, and either entered values or a formula shared by every row.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    /// The formula every row of this column computes, if it is a computed column.
    pub formula: Option<String>,
}

/// A sheet: named columns, rows of entered values, and the out-of-band constants.
#[derive(Debug, Clone, Default)]
pub struct Sheet {
    pub columns: Vec<Column>,
    /// `rows[r][c]` — what the user typed. A computed column's entries are overwritten by
    /// [`Sheet::evaluate`].
    pub rows: Vec<Vec<Value>>,
    /// `$$name` → value, from label/value pairs elsewhere in the document.
    pub constants: HashMap<String, f64>,
    /// Formulas for a totals row, by column name: `SUM($hours)`.
    pub totals: HashMap<String, String>,
}

/// The result of a calculation: the filled grid, plus the totals row when one was asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Calculated {
    pub rows: Vec<Vec<Value>>,
    pub totals: Vec<Value>,
}

impl Sheet {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// Calculate every computed column for every row, then the totals row.
    ///
    /// Column order in the document is irrelevant: dependencies decide the order of work, so a
    /// column may be written to the left of one it depends on.
    pub fn evaluate(&self) -> Calculated {
        let order = self.evaluation_order();
        let mut grid = self.rows.clone();
        for row in grid.iter_mut() {
            row.resize(self.columns.len(), Value::Empty);
        }

        for step in &order {
            match step {
                Step::Cycle(idx) => {
                    for row in grid.iter_mut() {
                        row[*idx] = Value::Error("#CYCLE");
                    }
                }
                Step::Column(idx) => {
                    let Some(formula) = self.columns[*idx].formula.clone() else { continue };
                    for r in 0..grid.len() {
                        let ctx = RowContext { sheet: self, grid: &grid, row: r };
                        grid[r][*idx] = eval(&formula, &ctx);
                    }
                }
            }
        }

        // Totals last: an aggregate reads finished columns, which is why it cannot be part of the
        // per-row ordering.
        let mut totals = Vec::new();
        if !self.totals.is_empty() {
            for col in &self.columns {
                match self.totals.get(&col.name) {
                    Some(f) => {
                        let ctx = RowContext { sheet: self, grid: &grid, row: usize::MAX };
                        totals.push(eval(f, &ctx));
                    }
                    None => totals.push(Value::Empty),
                }
            }
        }

        Calculated { rows: grid, totals }
    }

    /// Dependency-ordered steps, with any cycle reported rather than looped.
    fn evaluation_order(&self) -> Vec<Step> {
        let mut deps: Vec<HashSet<usize>> = vec![HashSet::new(); self.columns.len()];
        for (i, col) in self.columns.iter().enumerate() {
            let Some(f) = &col.formula else { continue };
            for name in referenced_columns(f) {
                if let Some(j) = self.column_index(&name) {
                    if j != i {
                        deps[i].insert(j);
                    }
                }
            }
        }

        let mut done: Vec<bool> = vec![false; self.columns.len()];
        let mut out = Vec::new();
        // Entered columns are ready immediately.
        for (i, col) in self.columns.iter().enumerate() {
            if col.formula.is_none() {
                done[i] = true;
                out.push(Step::Column(i));
            }
        }
        loop {
            let mut progressed = false;
            for i in 0..self.columns.len() {
                if done[i] {
                    continue;
                }
                if deps[i].iter().all(|d| done[*d]) {
                    done[i] = true;
                    out.push(Step::Column(i));
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
        // Anything still not done sits in a cycle (or depends on one).
        for i in 0..self.columns.len() {
            if !done[i] {
                out.push(Step::Cycle(i));
            }
        }
        out
    }
}

enum Step {
    Column(usize),
    Cycle(usize),
}

/// The column names a formula reads, ignoring `$$constants` and function names.
fn referenced_columns(formula: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = formula.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '$' {
            if i + 1 < bytes.len() && bytes[i + 1] == '$' {
                // a constant — skip its name entirely
                i += 2;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                continue;
            }
            i += 1;
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '_') {
                i += 1;
            }
            if i > start {
                out.push(bytes[start..i].iter().collect());
            }
            continue;
        }
        i += 1;
    }
    out
}

struct RowContext<'a> {
    sheet: &'a Sheet,
    grid: &'a [Vec<Value>],
    /// `usize::MAX` for the totals row, which has no row of its own to read.
    row: usize,
}

impl RowContext<'_> {
    fn column(&self, name: &str) -> Value {
        let Some(idx) = self.sheet.column_index(name) else { return Value::Error("#REF") };
        if self.row == usize::MAX {
            return Value::Error("#ROW");
        }
        self.grid
            .get(self.row)
            .and_then(|r| r.get(idx))
            .cloned()
            .unwrap_or(Value::Empty)
    }

    fn constant(&self, name: &str) -> Value {
        match self.sheet.constants.get(name) {
            Some(v) => Value::Number(*v),
            None => Value::Error("#REF"),
        }
    }

    fn aggregate(&self, func: &str, name: &str) -> Value {
        let Some(idx) = self.sheet.column_index(name) else { return Value::Error("#REF") };
        let nums: Vec<f64> = self
            .grid
            .iter()
            .filter_map(|r| r.get(idx).and_then(|v| v.as_number()))
            .collect();
        match func {
            "SUM" => Value::Number(nums.iter().sum()),
            "COUNT" => Value::Number(nums.len() as f64),
            "AVG" | "AVERAGE" => {
                if nums.is_empty() {
                    Value::Error("#DIV0")
                } else {
                    Value::Number(nums.iter().sum::<f64>() / nums.len() as f64)
                }
            }
            "MIN" => nums.iter().copied().fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.min(b))))
                .map(Value::Number)
                .unwrap_or(Value::Empty),
            "MAX" => nums.iter().copied().fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))))
                .map(Value::Number)
                .unwrap_or(Value::Empty),
            _ => Value::Error("#NAME"),
        }
    }
}

/// Evaluate one formula in one row's context.
fn eval(formula: &str, ctx: &RowContext<'_>) -> Value {
    let mut p = Parser { chars: formula.chars().collect(), pos: 0, ctx };
    p.skip_ws();
    // A leading '=' is allowed, because everyone types it.
    if p.peek() == Some('=') {
        p.pos += 1;
    }
    let v = p.expr();
    p.skip_ws();
    if p.pos < p.chars.len() {
        return Value::Error("#SYNTAX");
    }
    v
}

struct Parser<'a, 'b> {
    chars: Vec<char>,
    pos: usize,
    ctx: &'b RowContext<'a>,
}

impl Parser<'_, '_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn expr(&mut self) -> Value {
        let mut left = self.term();
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('+') => '+',
                Some('-') => '-',
                _ => return left,
            };
            self.pos += 1;
            let right = self.term();
            left = arith(left, right, op);
        }
    }

    fn term(&mut self) -> Value {
        let mut left = self.factor();
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('*') => '*',
                Some('/') => '/',
                _ => return left,
            };
            self.pos += 1;
            let right = self.factor();
            left = arith(left, right, op);
        }
    }

    fn factor(&mut self) -> Value {
        self.skip_ws();
        match self.peek() {
            Some('-') => {
                self.pos += 1;
                match self.factor() {
                    Value::Number(n) => Value::Number(-n),
                    other => other,
                }
            }
            Some('(') => {
                self.pos += 1;
                let v = self.expr();
                self.skip_ws();
                if self.peek() == Some(')') {
                    self.pos += 1;
                    v
                } else {
                    Value::Error("#SYNTAX")
                }
            }
            Some('$') => {
                self.pos += 1;
                let constant = self.peek() == Some('$');
                if constant {
                    self.pos += 1;
                }
                let name = self.ident();
                if name.is_empty() {
                    return Value::Error("#SYNTAX");
                }
                if constant {
                    self.ctx.constant(&name)
                } else {
                    self.ctx.column(&name)
                }
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.number(),
            Some(c) if c.is_ascii_alphabetic() => {
                let func = self.ident().to_ascii_uppercase();
                self.skip_ws();
                if self.peek() != Some('(') {
                    return Value::Error("#NAME");
                }
                self.pos += 1;
                self.skip_ws();
                // The argument is always a column reference: SUM($hours).
                if self.peek() != Some('$') {
                    return Value::Error("#SYNTAX");
                }
                self.pos += 1;
                if self.peek() == Some('$') {
                    return Value::Error("#SYNTAX"); // aggregating a constant is meaningless
                }
                let col = self.ident();
                self.skip_ws();
                if self.peek() != Some(')') {
                    return Value::Error("#SYNTAX");
                }
                self.pos += 1;
                self.ctx.aggregate(&func, &col)
            }
            _ => Value::Error("#SYNTAX"),
        }
    }

    fn ident(&mut self) -> String {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_') {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn number(&mut self) -> Value {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.') {
            self.pos += 1;
        }
        match self.chars[start..self.pos].iter().collect::<String>().parse() {
            Ok(n) => Value::Number(n),
            Err(_) => Value::Error("#SYNTAX"),
        }
    }
}

fn arith(a: Value, b: Value, op: char) -> Value {
    if let Value::Error(e) = a {
        return Value::Error(e);
    }
    if let Value::Error(e) = b {
        return Value::Error(e);
    }
    let (Some(x), Some(y)) = (a.as_number(), b.as_number()) else {
        return Value::Error("#VALUE");
    };
    match op {
        '+' => Value::Number(x + y),
        '-' => Value::Number(x - y),
        '*' => Value::Number(x * y),
        '/' => {
            if y == 0.0 {
                Value::Error("#DIV0")
            } else {
                Value::Number(x / y)
            }
        }
        _ => Value::Error("#SYNTAX"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payroll() -> Sheet {
        // Piers's brief, in full: entered columns, then every derived column, in an order
        // deliberately NOT matching the dependency order — the engine sorts it out.
        let cols = [
            ("Pay Rate", None),
            ("Hours", None),
            ("Super Amount", Some("$pay_rate * $$superannuation_rate")),
            ("Sub Total", Some("$pay_rate + $super_amount")),
            ("WC Amount", Some("$sub_total * $$workcover_rate")),
            ("PRT Amount", Some("$sub_total * $$payroll_tax_rate")),
            ("Total Cost Per Hour", Some("$sub_total + $wc_amount + $prt_amount")),
            ("Pay Amount", Some("$hours * $pay_rate")),
            ("Cost Amount", Some("$hours * $total_cost_per_hour")),
            ("Charge Rate", Some("$total_cost_per_hour * $$margin")),
            ("Revenue", Some("$charge_rate * $hours")),
            ("Gross Profit", Some("$revenue - $cost_amount")),
            ("Tax Amount", Some("$pay_amount * $$tax_rate")),
            ("Net Pay", Some("$pay_amount - $tax_amount")),
        ];
        let columns = cols
            .iter()
            .map(|(label, f)| Column { name: name_of(label), formula: f.map(|s| s.to_string()) })
            .collect();

        // Rates as they would sit in a label/value table elsewhere in the document.
        let constants = HashMap::from([
            ("superannuation_rate".to_string(), 0.115),
            ("workcover_rate".to_string(), 0.0275),
            ("payroll_tax_rate".to_string(), 0.0475),
            ("margin".to_string(), 1.65),
            ("tax_rate".to_string(), 0.325),
        ]);

        // Ten employees, $27–$65 as the brief asks.
        let rates = [27.5, 31.0, 34.25, 38.9, 42.0, 45.75, 50.0, 55.5, 60.25, 64.9];
        let rows = rates
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut row = vec![Value::Empty; 14];
                row[0] = Value::Number(*r);
                row[1] = Value::Number(38.0 + i as f64); // 38..47 hours
                row
            })
            .collect();

        let totals = HashMap::from([
            ("hours".to_string(), "SUM($hours)".to_string()),
            ("pay_amount".to_string(), "SUM($pay_amount)".to_string()),
            ("net_pay".to_string(), "SUM($net_pay)".to_string()),
            ("revenue".to_string(), "SUM($revenue)".to_string()),
            ("gross_profit".to_string(), "SUM($gross_profit)".to_string()),
        ]);

        Sheet { columns, rows, constants, totals }
    }

    fn col(sheet: &Sheet, out: &Calculated, row: usize, name: &str) -> f64 {
        let i = sheet.column_index(name).expect(name);
        out.rows[row][i].as_number().unwrap_or_else(|| panic!("{name} not a number: {:?}", out.rows[row][i]))
    }

    #[test]
    fn the_payroll_brief_computes_row_by_row() {
        let sheet = payroll();
        let out = sheet.evaluate();
        assert_eq!(out.rows.len(), 10);

        // First employee, worked by hand from the brief.
        let pay = 27.5;
        let super_amt = pay * 0.115;
        let sub = pay + super_amt;
        let wc = sub * 0.0275;
        let prt = sub * 0.0475;
        let tcph = sub + wc + prt;
        let hours = 38.0;
        let pay_amount = hours * pay;
        let charge = tcph * 1.65;
        let revenue = charge * hours;
        let cost = hours * tcph;

        assert!((col(&sheet, &out, 0, "super_amount") - super_amt).abs() < 1e-9);
        assert!((col(&sheet, &out, 0, "sub_total") - sub).abs() < 1e-9);
        assert!((col(&sheet, &out, 0, "total_cost_per_hour") - tcph).abs() < 1e-9);
        assert!((col(&sheet, &out, 0, "pay_amount") - pay_amount).abs() < 1e-9);
        assert!((col(&sheet, &out, 0, "cost_amount") - cost).abs() < 1e-9);
        assert!((col(&sheet, &out, 0, "charge_rate") - charge).abs() < 1e-9);
        assert!((col(&sheet, &out, 0, "revenue") - revenue).abs() < 1e-9);
        assert!((col(&sheet, &out, 0, "gross_profit") - (revenue - cost)).abs() < 1e-9);
        assert!((col(&sheet, &out, 0, "net_pay") - (pay_amount - pay_amount * 0.325)).abs() < 1e-9);
        // Every employee is profitable at a 1.65 margin — the sanity check the brief implies.
        for r in 0..10 {
            assert!(col(&sheet, &out, r, "gross_profit") > 0.0);
        }
    }

    #[test]
    fn the_totals_row_sums_the_finished_columns() {
        let sheet = payroll();
        let out = sheet.evaluate();
        let idx = |n: &str| sheet.column_index(n).unwrap();

        let hours: f64 = (38..48).map(|h| h as f64).sum();
        assert!((out.totals[idx("hours")].as_number().unwrap() - hours).abs() < 1e-9);

        let pay_sum: f64 = (0..10).map(|r| col(&sheet, &out, r, "pay_amount")).sum();
        assert!((out.totals[idx("pay_amount")].as_number().unwrap() - pay_sum).abs() < 1e-6);
        // A column with no total stays blank rather than showing a zero.
        assert_eq!(out.totals[idx("pay_rate")], Value::Empty);
    }

    #[test]
    fn changing_a_rate_changes_everything_downstream() {
        // "Change numbers & hit Recalc and it updates."
        let mut sheet = payroll();
        let before = sheet.evaluate();
        sheet.rows[0][0] = Value::Number(55.0); // the first employee's pay rate
        let after = sheet.evaluate();
        assert!(col(&sheet, &after, 0, "net_pay") > col(&sheet, &before, 0, "net_pay"));
        assert!(col(&sheet, &after, 0, "revenue") > col(&sheet, &before, 0, "revenue"));
        // and a constant moves every row
        sheet.constants.insert("superannuation_rate".into(), 0.12);
        let bumped = sheet.evaluate();
        for r in 0..10 {
            assert!(col(&sheet, &bumped, r, "super_amount") > col(&sheet, &after, r, "super_amount"));
        }
    }

    #[test]
    fn names_come_from_the_headings() {
        assert_eq!(name_of("Pay Rate"), "pay_rate");
        assert_eq!(name_of("Total Cost / Hour"), "total_cost_hour");
        assert_eq!(name_of("  Super Rate  "), "super_rate");
        assert_eq!(name_of("$$WorkCover Rate"), "workcover_rate");
    }

    #[test]
    fn errors_are_values_not_panics_and_not_zeros() {
        let mut sheet = payroll();
        sheet.columns.push(Column { name: "bad".into(), formula: Some("$nonexistent * 2".into()) });
        sheet.columns.push(Column { name: "boom".into(), formula: Some("$pay_rate / 0".into()) });
        sheet.columns.push(Column { name: "gibberish".into(), formula: Some("$pay_rate +".into()) });
        let out = sheet.evaluate();
        let i = |n: &str| sheet.column_index(n).unwrap();
        assert_eq!(out.rows[0][i("bad")], Value::Error("#REF"));
        assert_eq!(out.rows[0][i("boom")], Value::Error("#DIV0"));
        assert_eq!(out.rows[0][i("gibberish")], Value::Error("#SYNTAX"));
    }

    #[test]
    fn a_cycle_is_reported_rather_than_looped() {
        let columns = vec![
            Column { name: "a".into(), formula: Some("$b + 1".into()) },
            Column { name: "b".into(), formula: Some("$a + 1".into()) },
        ];
        let sheet = Sheet { columns, rows: vec![vec![Value::Empty, Value::Empty]], ..Default::default() };
        let out = sheet.evaluate();
        assert_eq!(out.rows[0][0], Value::Error("#CYCLE"));
        assert_eq!(out.rows[0][1], Value::Error("#CYCLE"));
    }

    #[test]
    fn four_decimal_places_in_and_out() {
        assert_eq!(Value::Number(11.5).display(), "11.5000");
        assert_eq!(Value::Number(1.0 / 3.0).display(), "0.3333");
        // A rate typed with a % or $ still reads as a number.
        assert_eq!(Value::Text("$27.50".into()).as_number(), Some(27.5));
    }
}
