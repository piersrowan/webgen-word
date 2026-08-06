//! Recalculating a document's tables.
//!
//! The bridge between the table model (`table.rs`) and the calculation engine (`webgen-calc`).
//! Piers's rule from HYBRID.md holds throughout: **the values live in the document as text**, so
//! a recipient with no WebGen, no JavaScript and no calculator still reads the right numbers.
//! Formulas are the recipe; the text is the meal.
//!
//! How a table becomes a sheet:
//!
//! - **Column names** come from the heading row, normalised (`"Pay Rate"` → `pay_rate`).
//! - **A computed column** is one whose body cells carry a formula. Every row of that column runs
//!   the same formula, which is how a spreadsheet behaves and how the brief was written.
//! - **The totals row** is the LAST body row when its cells carry aggregate formulas — `SUM(...)`
//!   — and it is excluded from the rows those aggregates run over, or a total would count itself.
//! - **Constants** (`$$superannuation_rate`) are gathered from every OTHER table in the document:
//!   any two-column row whose first cell is a label and whose second parses as a number. That is
//!   exactly the shape Piers described: `$A$1 = "Super Rate"`, `$B$1 = "11.5000"`.

use crate::table::{Cell, Table};
use std::collections::HashMap;
use webgen_calc::{name_of, Column, Sheet, Value};

/// Gather `$$name` → value from a rates table: any row of two-or-more cells whose first cell is
/// text and whose second is a number.
pub fn constants_from(tables: &[Table]) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for t in tables {
        for row in t.head.iter().chain(t.body.iter()).chain(t.foot.iter()) {
            if row.len() < 2 {
                continue;
            }
            let label = row[0].text.trim();
            if label.is_empty() {
                continue;
            }
            let Some(value) = Value::Text(row[1].text.clone()).as_number() else { continue };
            // A percentage written as "11.5%" means 0.115; one written as 0.115 means itself.
            let value = if row[1].text.contains('%') { value / 100.0 } else { value };
            out.insert(name_of(label), value);
        }
    }
    out
}

/// Whether this table has anything to calculate.
pub fn has_formulas(t: &Table) -> bool {
    t.body.iter().chain(t.foot.iter()).flatten().any(|c| !c.formula.is_empty())
}

/// Recalculate one table in place. Returns how many cells changed.
pub fn recalculate(t: &mut Table, constants: &HashMap<String, f64>) -> usize {
    if t.head.is_empty() || t.body.is_empty() {
        return 0;
    }
    let headings: Vec<String> = t.head[0].iter().map(|c| name_of(&c.text)).collect();
    if headings.iter().all(|h| h.is_empty()) {
        return 0;
    }

    // A trailing row of aggregates is the totals row, and must not be summed into itself.
    let last = t.body.len() - 1;
    let totals_row = t.body[last]
        .iter()
        .any(|c| c.formula.to_ascii_uppercase().contains("SUM(") || c.formula.to_ascii_uppercase().contains("AVG("));
    let data_rows = if totals_row { last } else { t.body.len() };

    // A column's formula is whichever formula its data cells carry (they agree in practice; the
    // first one wins, which keeps a hand-edited outlier from redefining the column).
    let mut columns = Vec::new();
    for (c, name) in headings.iter().enumerate() {
        let formula = (0..data_rows)
            .filter_map(|r| t.body[r].get(c))
            .map(|cell| cell.formula.clone())
            .find(|f| !f.is_empty());
        columns.push(Column { name: name.clone(), formula });
    }

    let rows: Vec<Vec<Value>> = (0..data_rows)
        .map(|r| {
            (0..headings.len())
                .map(|c| match t.body[r].get(c) {
                    Some(cell) if cell.text.trim().is_empty() => Value::Empty,
                    Some(cell) => Value::Text(cell.text.clone()),
                    None => Value::Empty,
                })
                .collect()
        })
        .collect();

    let totals: HashMap<String, String> = if totals_row {
        headings
            .iter()
            .enumerate()
            .filter_map(|(c, name)| {
                t.body[last]
                    .get(c)
                    .filter(|cell| !cell.formula.is_empty())
                    .map(|cell| (name.clone(), cell.formula.clone()))
            })
            .collect()
    } else {
        HashMap::new()
    };

    let sheet = Sheet { columns: columns.clone(), rows, constants: constants.clone(), totals };
    let out = sheet.evaluate();

    // Write the results back as TEXT, leaving the formulas untouched.
    let mut changed = 0;
    for r in 0..data_rows {
        for c in 0..headings.len() {
            let Some(cell) = t.body[r].get_mut(c) else { continue };
            if cell.formula.is_empty() {
                continue;
            }
            let text = out.rows[r][c].display();
            if cell.text != text {
                cell.text = text;
                changed += 1;
            }
        }
    }
    if totals_row {
        for c in 0..headings.len() {
            let Some(cell) = t.body[last].get_mut(c) else { continue };
            if cell.formula.is_empty() {
                continue;
            }
            let text = out.totals.get(c).map(|v| v.display()).unwrap_or_default();
            if cell.text != text {
                cell.text = text;
                changed += 1;
            }
        }
    }
    changed
}

/// Build the payroll worked example from Piers's brief: a rates table and ten employees.
/// Used by "Insert payroll example" so the feature can be tried without typing formulas.
pub fn payroll_example(first_id: u32) -> (Table, Table) {
    let rates = [
        ("Super Rate", "0.1150"),
        ("WorkCover Rate", "0.0275"),
        ("Payroll Tax Rate", "0.0475"),
        ("Margin", "1.6500"),
        ("Tax Rate", "0.3250"),
    ];
    let mut rates_table = Table::new(first_id, rates.len(), 2);
    rates_table.head[0][0] = Cell::with_text("Rate");
    rates_table.head[0][1] = Cell::with_text("Value");
    for (i, (label, value)) in rates.iter().enumerate() {
        rates_table.body[i][0] = Cell::with_text(label);
        rates_table.body[i][1] = Cell::with_text(value);
    }

    let columns: [(&str, &str); 14] = [
        ("Pay Rate", ""),
        ("Hours", ""),
        ("Super Amount", "$pay_rate * $$super_rate"),
        ("Sub Total", "$pay_rate + $super_amount"),
        ("WC Amount", "$sub_total * $$workcover_rate"),
        ("PRT Amount", "$sub_total * $$payroll_tax_rate"),
        ("Total Cost Per Hour", "$sub_total + $wc_amount + $prt_amount"),
        ("Pay Amount", "$hours * $pay_rate"),
        ("Cost Amount", "$hours * $total_cost_per_hour"),
        ("Charge Rate", "$total_cost_per_hour * $$margin"),
        ("Revenue", "$charge_rate * $hours"),
        ("Gross Profit", "$revenue - $cost_amount"),
        ("Tax Amount", "$pay_amount * $$tax_rate"),
        ("Net Pay", "$pay_amount - $tax_amount"),
    ];
    // Ten employees plus a totals row.
    let mut pay = Table::new(first_id + 1, 11, columns.len());
    for (c, (label, _)) in columns.iter().enumerate() {
        pay.head[0][c] = Cell { bold: true, ..Cell::with_text(label) };
    }
    let entered = [27.5, 31.0, 34.25, 38.9, 42.0, 45.75, 50.0, 55.5, 60.25, 64.9];
    for (r, rate) in entered.iter().enumerate() {
        for (c, (_, formula)) in columns.iter().enumerate() {
            pay.body[r][c] = Cell {
                formula: formula.to_string(),
                ..Cell::with_text(match c {
                    0 => format!("{rate:.4}"),
                    1 => format!("{:.4}", 38.0 + r as f64),
                    _ => String::new(),
                }
                .as_str())
            };
        }
    }
    let totals: [(usize, &str); 5] = [
        (1, "SUM($hours)"),
        (7, "SUM($pay_amount)"),
        (10, "SUM($revenue)"),
        (11, "SUM($gross_profit)"),
        (13, "SUM($net_pay)"),
    ];
    pay.body[10][0] = Cell { bold: true, ..Cell::with_text("Totals") };
    for (c, f) in totals {
        pay.body[10][c] = Cell { bold: true, formula: f.to_string(), ..Cell::default() };
    }
    (rates_table, pay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payroll_example_calculates_and_totals() {
        let (rates, mut pay) = payroll_example(1);
        let constants = constants_from(&[rates]);
        assert_eq!(constants.get("super_rate"), Some(&0.115));
        assert!(has_formulas(&pay));

        let changed = recalculate(&mut pay, &constants);
        assert!(changed > 100, "expected a full grid to fill, changed {changed}");

        // Employee 1: 27.50/h, 38 hours.
        let val = |r: usize, c: usize| -> f64 { pay.body[r][c].text.parse().unwrap_or(f64::NAN) };
        assert!((val(0, 2) - 27.5 * 0.115).abs() < 1e-4, "super {}", val(0, 2));
        assert!((val(0, 7) - 27.5 * 38.0).abs() < 1e-4, "pay {}", val(0, 7));
        assert!(val(0, 11) > 0.0, "gross profit positive");
        // The totals row sums the ten employees and not itself.
        let hours: f64 = (38..48).map(|h| h as f64).sum();
        assert!((val(10, 1) - hours).abs() < 1e-4, "hours total {}", val(10, 1));
        // Every value is written as TEXT in the document, formulas untouched.
        assert!(!pay.body[0][2].text.is_empty());
        assert_eq!(pay.body[0][2].formula, "$pay_rate * $$super_rate");
    }

    #[test]
    fn changing_an_entered_value_moves_the_results() {
        let (rates, mut pay) = payroll_example(1);
        let constants = constants_from(&[rates]);
        recalculate(&mut pay, &constants);
        let before: f64 = pay.body[0][13].text.parse().unwrap();
        pay.body[0][0].text = "55.0000".into(); // the pay rate someone typed
        recalculate(&mut pay, &constants);
        let after: f64 = pay.body[0][13].text.parse().unwrap();
        assert!(after > before, "net pay should follow the rate: {before} -> {after}");
    }

    #[test]
    fn a_percentage_rate_is_read_as_a_fraction() {
        let mut rates = Table::new(1, 1, 2);
        rates.body[0][0] = Cell::with_text("Super Rate");
        rates.body[0][1] = Cell::with_text("11.5%");
        assert_eq!(constants_from(&[rates]).get("super_rate"), Some(&0.115));
    }
}
