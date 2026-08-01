//! The Table Window — where **all** table work happens.
//!
//! The process Piers set out: *"Document > Table insert/select > All activity Managed in Table
//! Window > Save / Delete > The content between the table boundary markers is deleted and replaced
//! with the generated HTML/CSS code."*
//!
//! So this window owns the table completely while it is open. Nothing edits a table in the document
//! itself — not the text, not the structure, not the style. That is deliberate: a table is
//! two-dimensional and the document is a stream, and every table editor that has tried to be both
//! at once has ended up with cells you cannot select and merges you cannot undo. Here the model is
//! the truth, the window edits the model, and Save regenerates the block from it.
//!
//! The left half is the grid, the right half is the CSS — the same property rows the document style
//! sidebar uses ([`crate::stylerows`]), pointed at table selectors instead of document tags.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::docstyle::TagStyle;
use crate::settings::Settings;
use crate::stylerows::StyleRows;
use crate::table::{Cell, Grid, Table, SELECTORS};

/// What the window was closed with.
pub enum Outcome {
    Save(Table),
    Delete,
}

/// Where the cursor is in the editor. Grid coordinates, not array indices — the array index of a
/// cell depends on what is merged above and to the left of it.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Spot {
    head: bool,
    row: usize,
    col: usize,
}

impl Default for Spot {
    fn default() -> Self {
        Spot { head: true, row: 0, col: 0 }
    }
}

struct Ui {
    table: RefCell<Table>,
    spot: std::cell::Cell<Spot>,
    /// Set while the toolbar is being made to reflect the current cell, so doing so is not a change.
    syncing: std::cell::Cell<bool>,
    /// The style selector the CSS panel is showing.
    selector: RefCell<String>,
}

impl Ui {
    /// The rows of the section the cursor is in.
    fn rows(&self) -> Vec<Vec<Cell>> {
        let t = self.table.borrow();
        if self.spot.get().head { t.head.clone() } else { t.body.clone() }
    }

    /// The array index of the anchor cell under the cursor, if there is one.
    fn anchor(&self) -> Option<(usize, usize)> {
        let spot = self.spot.get();
        let rows = self.rows();
        Grid::of(&rows).at(spot.row, spot.col).map(|p| (p.row, p.index))
    }

    fn with_cell<R>(&self, f: impl FnOnce(&mut Cell) -> R) -> Option<R> {
        let (row, index) = self.anchor()?;
        let head = self.spot.get().head;
        let mut t = self.table.borrow_mut();
        let section = if head { &mut t.head } else { &mut t.body };
        section.get_mut(row)?.get_mut(index).map(f)
    }

    fn cell(&self) -> Option<Cell> {
        let (row, index) = self.anchor()?;
        let head = self.spot.get().head;
        let t = self.table.borrow();
        let section = if head { &t.head } else { &t.body };
        section.get(row)?.get(index).cloned()
    }
}

/// Open the window on `table`. `done` is called with what the document should do about it; closing
/// or cancelling calls nothing at all.
pub fn open(
    parent: &adw::ApplicationWindow,
    settings: &Rc<Settings>,
    table: Table,
    is_new: bool,
    done: Rc<dyn Fn(Outcome)>,
) {
    let ui = Rc::new(Ui {
        table: RefCell::new(table),
        spot: std::cell::Cell::new(Spot::default()),
        syncing: std::cell::Cell::new(false),
        selector: RefCell::new(SELECTORS[0].0.to_string()),
    });

    let window = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(if is_new { "New table" } else { "Table" })
        .default_width(1080)
        .default_height(660)
        .build();

    // --- header -------------------------------------------------------------------------------
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    let cancel_b = gtk::Button::with_label("Cancel");
    let save_b = gtk::Button::with_label("Save");
    save_b.add_css_class("suggested-action");
    let delete_b = gtk::Button::with_label("Delete table");
    delete_b.add_css_class("destructive-action");
    header.pack_start(&cancel_b);
    header.pack_end(&save_b);
    if !is_new {
        header.pack_end(&delete_b);
    }

    // --- the grid editor ------------------------------------------------------------------------
    let grid = gtk::Grid::new();
    grid.set_row_spacing(2);
    grid.set_column_spacing(2);
    grid.set_margin_top(12);
    grid.set_margin_bottom(12);
    grid.set_margin_start(12);
    grid.set_margin_end(12);

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.set_margin_top(8);
    bar.set_margin_start(12);
    bar.set_margin_end(12);

    let tool = |icon: &str, tip: &str| {
        let b = gtk::Button::from_icon_name(icon);
        b.set_tooltip_text(Some(tip));
        b.add_css_class("flat");
        b
    };
    let row_above = tool("list-add-symbolic", "Insert a row above");
    let row_below = tool("list-add-symbolic", "Insert a row below");
    let row_del = tool("list-remove-symbolic", "Delete this row");
    let col_left = tool("list-add-symbolic", "Insert a column to the left");
    let col_right = tool("list-add-symbolic", "Insert a column to the right");
    let col_del = tool("list-remove-symbolic", "Delete this column");
    let merge_right = tool("go-next-symbolic", "Merge with the cell to the right");
    let merge_down = tool("go-down-symbolic", "Merge with the cell below");
    let split = tool("edit-cut-symbolic", "Split this merged cell back up");

    let bold_b = gtk::ToggleButton::new();
    bold_b.set_icon_name("format-text-bold-symbolic");
    bold_b.set_tooltip_text(Some("Bold"));
    bold_b.add_css_class("flat");
    let italic_b = gtk::ToggleButton::new();
    italic_b.set_icon_name("format-text-italic-symbolic");
    italic_b.set_tooltip_text(Some("Italic"));
    italic_b.add_css_class("flat");
    let under_b = gtk::ToggleButton::new();
    under_b.set_icon_name("format-text-underline-symbolic");
    under_b.set_tooltip_text(Some("Underline"));
    under_b.add_css_class("flat");

    let align_l = gtk::ToggleButton::new();
    align_l.set_icon_name("format-justify-left-symbolic");
    align_l.set_tooltip_text(Some("Align left"));
    align_l.add_css_class("flat");
    let align_c = gtk::ToggleButton::new();
    align_c.set_icon_name("format-justify-center-symbolic");
    align_c.set_tooltip_text(Some("Centre"));
    align_c.add_css_class("flat");
    align_c.set_group(Some(&align_l));
    let align_r = gtk::ToggleButton::new();
    align_r.set_icon_name("format-justify-right-symbolic");
    align_r.set_tooltip_text(Some("Align right"));
    align_r.add_css_class("flat");
    align_r.set_group(Some(&align_l));

    let sep = || {
        let s = gtk::Separator::new(gtk::Orientation::Vertical);
        s.set_margin_start(4);
        s.set_margin_end(4);
        s
    };
    for w in [&row_above, &row_below, &row_del] {
        bar.append(w);
    }
    bar.append(&sep());
    for w in [&col_left, &col_right, &col_del] {
        bar.append(w);
    }
    bar.append(&sep());
    for w in [&merge_right, &merge_down, &split] {
        bar.append(w);
    }
    bar.append(&sep());
    bar.append(&bold_b);
    bar.append(&italic_b);
    bar.append(&under_b);
    bar.append(&sep());
    bar.append(&align_l);
    bar.append(&align_c);
    bar.append(&align_r);

    let where_label = gtk::Label::new(None);
    where_label.add_css_class("dim-label");
    where_label.set_xalign(0.0);
    where_label.set_margin_start(12);
    where_label.set_margin_top(4);

    let grid_scroller = gtk::ScrolledWindow::builder().child(&grid).vexpand(true).hexpand(true).build();

    let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left.append(&bar);
    left.append(&where_label);
    left.append(&grid_scroller);

    // --- the CSS panel --------------------------------------------------------------------------
    let selector_row = adw::ComboRow::new();
    selector_row.set_title("Style");
    selector_row.set_model(Some(&gtk::StringList::new(
        &SELECTORS.iter().map(|(_, label)| *label).collect::<Vec<_>>(),
    )));
    let selector_group = adw::PreferencesGroup::new();
    selector_group.set_description(Some(
        "Applies to this table only — the rules are written scoped to it.",
    ));
    selector_group.add(&selector_row);

    let rows = StyleRows::new(settings);
    let css_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    css_box.set_margin_top(12);
    css_box.set_margin_start(10);
    css_box.set_margin_end(10);
    css_box.set_margin_bottom(12);
    css_box.append(&selector_group);
    css_box.append(&rows.group);
    let css_scroller = gtk::ScrolledWindow::builder().child(&css_box).vexpand(true).build();
    css_scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    css_scroller.set_max_content_width(330);
    css_box.set_size_request(310, -1);

    let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
    right.set_size_request(330, -1);
    right.set_hexpand(false);
    right.append(&css_scroller);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&left);
    body.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    body.append(&right);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    window.set_content(Some(&view));

    // --- keeping the widgets and the model in step ----------------------------------------------
    let rows = Rc::new(rows);

    // Reflect the cell under the cursor in the toolbar and the caption.
    let sync_toolbar: Rc<dyn Fn()> = {
        let ui = ui.clone();
        let (bold_b, italic_b, under_b) = (bold_b.clone(), italic_b.clone(), under_b.clone());
        let (align_l, align_c, align_r) = (align_l.clone(), align_c.clone(), align_r.clone());
        let where_label = where_label.clone();
        let split = split.clone();
        Rc::new(move || {
            let cell = ui.cell().unwrap_or_default();
            ui.syncing.set(true);
            bold_b.set_active(cell.bold);
            italic_b.set_active(cell.italic);
            under_b.set_active(cell.underline);
            align_l.set_active(cell.align == "left");
            align_c.set_active(cell.align == "center");
            align_r.set_active(cell.align == "right");
            ui.syncing.set(false);
            split.set_sensitive(cell.colspan > 1 || cell.rowspan > 1);
            let spot = ui.spot.get();
            where_label.set_text(&format!(
                "{} row {}, column {}{}",
                if spot.head { "Heading" } else { "Body" },
                spot.row + 1,
                spot.col + 1,
                if cell.colspan > 1 || cell.rowspan > 1 {
                    format!("  —  merged {}×{}", cell.rowspan, cell.colspan)
                } else {
                    String::new()
                }
            ));
        })
    };

    // Rebuild the whole editor from the model. Cheap, and it is the only way a merge or a deleted
    // column can be shown correctly without tracking widget identity through a reshape.
    let rebuild: Rc<dyn Fn()> = {
        let ui = ui.clone();
        let grid = grid.clone();
        let sync_toolbar = sync_toolbar.clone();
        Rc::new(move || {
            while let Some(child) = grid.first_child() {
                grid.remove(&child);
            }
            let (head, body) = {
                let t = ui.table.borrow();
                (t.head.clone(), t.body.clone())
            };
            let head_height = head.len();

            for (head_section, rows_of) in [(true, &head), (false, &body)] {
                let layout = Grid::of(rows_of);
                for placed in &layout.cells {
                    let cell = &rows_of[placed.row][placed.index];
                    let entry = gtk::Entry::new();
                    entry.set_text(&cell.text);
                    entry.set_hexpand(true);
                    entry.set_width_chars(10);
                    if head_section {
                        entry.add_css_class("heading");
                        entry.set_placeholder_text(Some("Heading"));
                    }
                    let display_row =
                        if head_section { placed.row } else { head_height + placed.row } as i32;
                    grid.attach(
                        &entry,
                        placed.col as i32,
                        display_row,
                        placed.colspan as i32,
                        placed.rowspan as i32,
                    );

                    // Typing writes straight through to the model.
                    {
                        let ui = ui.clone();
                        let spot = Spot { head: head_section, row: placed.row, col: placed.col };
                        entry.connect_changed(move |e| {
                            let previous = ui.spot.get();
                            ui.spot.set(spot);
                            ui.with_cell(|c| c.text = e.text().to_string());
                            ui.spot.set(previous);
                        });
                    }
                    // Clicking or tabbing into a cell is what selects it.
                    {
                        let ui = ui.clone();
                        let sync_toolbar = sync_toolbar.clone();
                        let spot = Spot { head: head_section, row: placed.row, col: placed.col };
                        let focus = gtk::EventControllerFocus::new();
                        focus.connect_enter(move |_| {
                            ui.spot.set(spot);
                            sync_toolbar();
                        });
                        entry.add_controller(focus);
                    }
                }
            }
            sync_toolbar();
        })
    };

    // --- structural buttons ----------------------------------------------------------------------
    macro_rules! structural {
        ($button:expr, $body:expr) => {{
            let ui = ui.clone();
            let rebuild = rebuild.clone();
            #[allow(clippy::redundant_closure_call)]
            $button.connect_clicked(move |_| {
                {
                    let spot = ui.spot.get();
                    let mut t = ui.table.borrow_mut();
                    #[allow(clippy::redundant_closure_call)]
                    ($body)(&mut *t, spot);
                }
                rebuild();
            });
        }};
    }

    structural!(row_above, |t: &mut Table, s: Spot| if !s.head {
        t.insert_row(s.row)
    });
    structural!(row_below, |t: &mut Table, s: Spot| if !s.head {
        t.insert_row(s.row + 1)
    } else {
        t.insert_row(0)
    });
    structural!(row_del, |t: &mut Table, s: Spot| if !s.head {
        t.delete_row(s.row)
    });
    structural!(col_left, |t: &mut Table, s: Spot| t.insert_column(s.col));
    structural!(col_right, |t: &mut Table, s: Spot| t.insert_column(s.col + 1));
    structural!(col_del, |t: &mut Table, s: Spot| t.delete_column(s.col));
    structural!(merge_right, |t: &mut Table, s: Spot| if !s.head {
        t.merge(s.row, s.col, s.row, s.col + 1);
    });
    structural!(merge_down, |t: &mut Table, s: Spot| if !s.head {
        t.merge(s.row, s.col, s.row + 1, s.col);
    });
    structural!(split, |t: &mut Table, s: Spot| if !s.head {
        t.split(s.row, s.col);
    });

    // --- cell formatting --------------------------------------------------------------------------
    for (button, setter) in [
        (&bold_b, 0usize),
        (&italic_b, 1),
        (&under_b, 2),
    ] {
        let ui = ui.clone();
        button.connect_toggled(move |b| {
            if ui.syncing.get() {
                return;
            }
            let on = b.is_active();
            ui.with_cell(|c| match setter {
                0 => c.bold = on,
                1 => c.italic = on,
                _ => c.underline = on,
            });
        });
    }
    for (button, value) in [(&align_l, "left"), (&align_c, "center"), (&align_r, "right")] {
        let ui = ui.clone();
        let value = value.to_string();
        button.connect_toggled(move |b| {
            if ui.syncing.get() {
                return;
            }
            let value = if b.is_active() { value.clone() } else { String::new() };
            ui.with_cell(|c| c.align = value);
        });
    }

    // --- the CSS panel ----------------------------------------------------------------------------
    let load_style: Rc<dyn Fn()> = {
        let (ui, rows) = (ui.clone(), rows.clone());
        Rc::new(move || {
            let selector = ui.selector.borrow().clone();
            let style = ui.table.borrow().css.get(&selector).cloned().unwrap_or_default();
            rows.load(&style);
        })
    };
    let store_style: Rc<dyn Fn()> = {
        let (ui, rows) = (ui.clone(), rows.clone());
        Rc::new(move || {
            let selector = ui.selector.borrow().clone();
            let style = rows.collect();
            let mut t = ui.table.borrow_mut();
            if style == TagStyle::default() {
                t.css.remove(&selector);
            } else {
                t.css.insert(selector, style);
            }
        })
    };
    {
        let (ui, load_style, store_style) = (ui.clone(), load_style.clone(), store_style.clone());
        selector_row.connect_selected_notify(move |row| {
            // The selector the panel was showing keeps whatever was set on it before moving on.
            store_style();
            let next = SELECTORS
                .get(row.selected() as usize)
                .map(|(s, _)| s.to_string())
                .unwrap_or_else(|| SELECTORS[0].0.to_string());
            *ui.selector.borrow_mut() = next;
            load_style();
        });
    }
    load_style();
    rebuild();

    // --- leaving -----------------------------------------------------------------------------------
    {
        let window = window.clone();
        cancel_b.connect_clicked(move |_| window.close());
    }
    {
        let (window, ui, done, store_style) =
            (window.clone(), ui.clone(), done.clone(), store_style.clone());
        save_b.connect_clicked(move |_| {
            store_style();
            let table = ui.table.borrow().clone();
            window.close();
            done(Outcome::Save(table));
        });
    }
    {
        let (window, done) = (window.clone(), done.clone());
        delete_b.connect_clicked(move |_| {
            let confirm = adw::MessageDialog::new(
                Some(&window),
                Some("Delete this table?"),
                Some("The table and its style go with it. This cannot be undone from here."),
            );
            confirm.add_response("delete", "Delete table");
            confirm.add_response("cancel", "Cancel");
            confirm.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
            confirm.set_default_response(Some("cancel"));
            confirm.set_close_response("cancel");
            let (window, done) = (window.clone(), done.clone());
            confirm.connect_response(None, move |dlg, response| {
                dlg.close();
                if response == "delete" {
                    window.close();
                    done(Outcome::Delete);
                }
            });
            confirm.present();
        });
    }

    window.present();
}

/// Ask how big a new table should be, then open the editor on it.
pub fn ask_size(
    parent: &adw::ApplicationWindow,
    settings: &Rc<Settings>,
    id: u32,
    done: Rc<dyn Fn(Outcome)>,
) {
    let dialog = adw::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("New table")
        .default_width(360)
        .build();

    let group = adw::PreferencesGroup::new();
    group.set_description(Some("A heading row is added on top of these."));
    let rows = adw::SpinRow::with_range(1.0, 100.0, 1.0);
    rows.set_title("Rows");
    rows.set_value(3.0);
    group.add(&rows);
    let cols = adw::SpinRow::with_range(1.0, 30.0, 1.0);
    cols.set_title("Columns");
    cols.set_value(3.0);
    group.add(&cols);

    let cancel = gtk::Button::with_label("Cancel");
    let create = gtk::Button::with_label("Create");
    create.add_css_class("suggested-action");
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let (dialog, parent, settings, done) =
            (dialog.clone(), parent.clone(), settings.clone(), done.clone());
        let (rows, cols) = (rows.clone(), cols.clone());
        create.connect_clicked(move |_| {
            let table = Table::new(id, rows.value() as usize, cols.value() as usize);
            dialog.close();
            open(&parent, &settings, table, true, done.clone());
        });
    }

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&create);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.append(&group);
    content.append(&buttons);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&content));
    dialog.set_content(Some(&view));
    dialog.present();
}
