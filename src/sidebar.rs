//! The element style sidebar: click something, style it.
//!
//! ## The scoping model, which is the whole point
//!
//! Piers, 2026-08-01: *"CSS targets as toggle (default) all instances of element vs this instance.
//! I could click one image and give it a red border. All images now have a red border. I could
//! toggle this to 'this instance' and change its border to green. This instance has an overrode one
//! line CSS entry for green border, the rest remain red."*
//!
//! So every edit lands on one of two selectors:
//!
//! - **All instances** (the default) — a bare tag rule, `img { border: 1px solid #cc0000; }`.
//! - **This instance** — a rule keyed to that element alone, `.wg-i1 { border: 1px solid #00aa00; }`.
//!   The element gets a minted `wg-iN` class if it has no `id` of its own to use.
//!
//! An instance rule beats the tag rule by CSS specificity, so the override is **exactly** the
//! properties it names. The green picture keeps everything else the red rule gave it.
//!
//! Two consequences of the rule, both implemented here:
//!
//! - **Opening on an element that is already styled specifically starts on "this instance"** — a
//!   minted handle, or an id or class of its own that some stylesheet actually targets.
//! - **Toggling back to "all instances" shows the page-wide values, and on Apply deletes the
//!   element-specific rule**, leaving the page-wide one to apply. The minted class comes off the
//!   element too, so dead handles do not accumulate. An `id` is left alone — it is the document's
//!   own and may mean something to somebody; only its rule in our block goes.
//!
//! ## No inline styles
//!
//! Nothing here writes a `style=""` attribute. Everything is a rule in the document's
//! `webgen-doc-custom` block, which is what keeps a document readable, restyleable in one place, and
//! the same shape the browser's editor understands.
//!
//! ## Navigating
//!
//! The title shows the resolved element — `<LI>` — and the arrows walk the tree: up to the parent,
//! down to the first element child. Given `<ul><li><span>Test</span></li></ul>`, clicking the text
//! resolves to `<li>`; up lands on `<ul>`, down on `<span>`.
//!
//! The selected element is marked in the DOM with a class rather than held as a handle in Rust,
//! because WebKit gives us no node handles — everything crosses that boundary as a string, so the
//! document itself has to remember what is selected between one call and the next.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use webkit6::prelude::*;

use crate::docstyle::{self, Selected, TagStyle};
use crate::settings::Settings;
use crate::State;

/// Wide enough for a colour chip and a font button, narrow enough to leave the document the window.
const SIDEBAR_WIDTH: i32 = 300;

const WEIGHTS: &[&str] = &["—", "normal", "bold"];
const SLANTS: &[&str] = &["—", "normal", "italic"];
const DECORATIONS: &[&str] = &["—", "none", "underline", "line-through"];
const FLOATS: &[&str] = &["—", "left", "right", "none"];
const ALIGNS: &[&str] = &["—", "left", "center", "right", "justify"];

/// The sidebar, and the handles the window needs to drive it.
pub struct Sidebar {
    /// Put this in the window's layout.
    pub root: gtk::Revealer,
    /// Select whatever is at these **widget** coordinates on the WebView and show its style.
    pub select_at: Rc<dyn Fn(f64, f64)>,
    /// Show or hide, without changing what is selected.
    pub set_open: Rc<dyn Fn(bool)>,
    /// Whether it is currently showing.
    pub is_open: Rc<dyn Fn() -> bool>,
    /// Re-read the selected element and its style from the document. Undo and redo change the
    /// overrides underneath the panel, and rows left showing the old values would re-apply them.
    pub refresh: Rc<dyn Fn()>,
}

/// Everything the panel needs to remember between one asynchronous DOM answer and the next.
struct Ui {
    view: webkit6::WebView,
    state: Rc<RefCell<State>>,
    selected: RefCell<Option<Selected>>,
    /// False = all instances (the default), true = this instance.
    instance_scope: Cell<bool>,
    /// Set while rows are being filled in, so doing so does not count as a choice.
    loading: Cell<bool>,
}

impl Ui {
    /// The selector the panel is currently editing, and whether it is ready to be written to.
    ///
    /// `None` in instance scope means the element has no handle yet — one is minted on Apply, so
    /// there is nothing to read values from and the rows open empty. That is correct: an element
    /// with no override of its own is not overriding anything.
    fn key(&self) -> Option<String> {
        let selected = self.selected.borrow();
        let s = selected.as_ref()?;
        if self.instance_scope.get() {
            (!s.instance.is_empty()).then(|| s.instance.clone())
        } else {
            Some(s.tag.clone())
        }
    }
}

pub fn build(
    view: &webkit6::WebView,
    state: &Rc<RefCell<State>>,
    settings: &Rc<Settings>,
) -> Sidebar {
    let ui = Rc::new(Ui {
        view: view.clone(),
        state: state.clone(),
        selected: RefCell::new(None),
        instance_scope: Cell::new(false),
        loading: Cell::new(false),
    });

    // --- header: navigation and the resolved element ------------------------------------------
    let up = gtk::Button::from_icon_name("go-up-symbolic");
    up.set_tooltip_text(Some("Select the element that contains this one"));
    up.add_css_class("flat");
    let down = gtk::Button::from_icon_name("go-down-symbolic");
    down.set_tooltip_text(Some("Select the first element inside this one"));
    down.add_css_class("flat");

    let title = gtk::Label::new(Some("<>"));
    title.add_css_class("heading");
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.set_tooltip_text(Some("Close the style sidebar"));
    close.add_css_class("flat");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    header.set_margin_top(8);
    header.set_margin_start(8);
    header.set_margin_end(8);
    header.append(&up);
    header.append(&title);
    header.append(&down);
    header.append(&close);

    // --- the scope toggle ----------------------------------------------------------------------
    let all_b = gtk::ToggleButton::with_label("All");
    all_b.set_tooltip_text(Some("Style every element of this kind in the document"));
    all_b.set_hexpand(true);
    let one_b = gtk::ToggleButton::with_label("This one");
    one_b.set_tooltip_text(Some("Style only the selected element, on top of the rule above"));
    one_b.set_hexpand(true);
    one_b.set_group(Some(&all_b));
    all_b.set_active(true);

    let scope = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    scope.add_css_class("linked");
    scope.set_margin_top(8);
    scope.set_margin_start(8);
    scope.set_margin_end(8);
    scope.append(&all_b);
    scope.append(&one_b);

    // "Apply to the cells inside" — the fix for styling a ROW and having nothing visible happen
    // (Piers, 2026-08-07). Only meaningful on a container that holds cells, so it is enabled by
    // `show_selected` and ignored everywhere else.
    let cells_b = gtk::CheckButton::with_label("Apply to the cells inside");
    cells_b.set_tooltip_text(Some(
        "Style every cell in the selected row, section or table, rather than the container itself",
    ));
    cells_b.set_margin_start(10);
    cells_b.set_margin_end(10);
    cells_b.set_margin_top(6);

    let scope_note = gtk::Label::new(None);
    scope_note.add_css_class("dim-label");
    scope_note.add_css_class("caption");
    scope_note.set_wrap(true);
    scope_note.set_xalign(0.0);
    scope_note.set_margin_start(10);
    scope_note.set_margin_end(10);
    scope_note.set_margin_top(4);

    // --- the properties, in two tabs (Piers, 2026-08-06) ----------------------------------------
    // "Type" is everything about the letters; "Box" is everything about the container. Same rows,
    // same Apply, same scoping — the tabs only organise. A third tab waits for the
    // others-that-got-forgotten.
    let g_type = adw::PreferencesGroup::new();
    let g_box = adw::PreferencesGroup::new();
    let group = g_type.clone(); // the typography rows land here

    let font = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    font.set_valign(gtk::Align::Center);
    let font_touched = Rc::new(Cell::new(false));
    {
        let (font_touched, ui) = (font_touched.clone(), ui.clone());
        font.connect_font_desc_notify(move |_| {
            if !ui.loading.get() {
                font_touched.set(true);
            }
        });
    }
    let font_row = adw::ActionRow::new();
    font_row.set_title("Font");
    font_row.add_suffix(&font);
    group.add(&font_row);

    // Bold / italic / underline. Combos rather than switches because "say nothing" and "explicitly
    // normal" are different answers: an explicit `normal` is how you take the bold off an element
    // that inherits it, and a switch cannot express the difference.
    let weight = adw::ComboRow::new();
    weight.set_title("Weight");
    weight.set_model(Some(&gtk::StringList::new(WEIGHTS)));
    group.add(&weight);

    let slant = adw::ComboRow::new();
    slant.set_title("Italic");
    slant.set_model(Some(&gtk::StringList::new(SLANTS)));
    group.add(&slant);

    let decoration = adw::ComboRow::new();
    decoration.set_title("Underline");
    decoration.set_model(Some(&gtk::StringList::new(DECORATIONS)));
    group.add(&decoration);

    // Kerning and leading — the two knobs Piers named that the panel lacked (2026-08-06).
    let letter_spacing = length_row("Letter spacing", "0 says nothing", &group);
    let line_height = adw::SpinRow::with_range(0.0, 3.0, 0.05);
    line_height.set_digits(2);
    line_height.set_title("Line height");
    line_height.set_subtitle("Multiplier; 0 says nothing");
    group.add(&line_height);

    let text_colour = ColourRow::new("Text colour", settings, "#1a1a1a");
    group.add(&text_colour.row);
    let background = ColourRow::new("Background", settings, "#ffffff");
    g_box.add(&background.row);

    // Border: three pickers, not a string to get wrong. **Width 0 removes the declaration
    // entirely** rather than writing `border: none` -- Piers' rule, and the one that leaves the
    // stanza clean.
    let border_width = adw::SpinRow::with_range(0.0, 40.0, 1.0);
    border_width.set_title("Border width");
    border_width.set_subtitle("0 removes the border");
    g_box.add(&border_width);

    let border_style = adw::ComboRow::new();
    border_style.set_title("Border style");
    border_style.set_model(Some(&gtk::StringList::new(docstyle::BORDER_STYLES)));
    g_box.add(&border_style);

    let border_colour = ColourRow::new("Border colour", settings, "#000000");
    g_box.add(&border_colour.row);

    let radius = adw::SpinRow::with_range(0.0, 60.0, 1.0);
    radius.set_title("Corner radius");
    g_box.add(&radius);

    let shadow = adw::SwitchRow::new();
    shadow.set_title("Drop shadow");
    g_box.add(&shadow);

    // Numbers with units, not free text -- see `stylerows::LengthRow` for why.
    let padding = length_row("Padding", "One value for all four sides", &g_box);
    let margin = length_row("Margin", "One value for all four sides", &g_box);
    let width = length_row("Width", "0 leaves the width to the content", &g_box);

    let float = adw::ComboRow::new();
    float.set_title("Float");
    float.set_subtitle("Text wraps the other side");
    float.set_model(Some(&gtk::StringList::new(FLOATS)));
    g_box.add(&float);

    let align = adw::ComboRow::new();
    align.set_title("Alignment");
    align.set_model(Some(&gtk::StringList::new(ALIGNS)));
    group.add(&align);

    let tabs = gtk::Stack::new();
    tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    tabs.add_titled(&g_type, Some("type"), "Type");
    tabs.add_titled(&g_box, Some("box"), "Box");
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&tabs));
    switcher.set_halign(gtk::Align::Center);

    let rows_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    rows_box.set_margin_top(12);
    rows_box.set_margin_start(8);
    rows_box.set_margin_end(8);
    rows_box.append(&switcher);
    rows_box.append(&tabs);
    let _ = &group; // g_type by its working alias; the rows above filled it
    let scroller = gtk::ScrolledWindow::builder().child(&rows_box).vexpand(true).build();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_propagate_natural_width(false);
    scroller.set_hexpand(false);
    // Never means "no horizontal scrollbar", which makes the ScrolledWindow demand its child's full
    // minimum width. Capping it here is what actually holds the panel to SIDEBAR_WIDTH.
    scroller.set_max_content_width(SIDEBAR_WIDTH);
    rows_box.set_size_request(SIDEBAR_WIDTH - 24, -1);

    let apply_b = gtk::Button::with_label("Apply");
    apply_b.add_css_class("suggested-action");
    apply_b.set_hexpand(true);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_margin_top(8);
    buttons.set_margin_bottom(8);
    buttons.set_margin_start(8);
    buttons.set_margin_end(8);
    buttons.append(&apply_b);

    let hint = gtk::Label::new(Some("Click anything in the document to style it."));
    hint.add_css_class("dim-label");
    hint.set_wrap(true);
    hint.set_margin_top(24);
    hint.set_margin_start(16);
    hint.set_margin_end(16);

    // The body swaps between the hint and the rows, so an empty panel explains itself.
    let body = gtk::Stack::new();
    body.set_vexpand(true);
    body.add_named(&hint, Some("hint"));
    let editing = gtk::Box::new(gtk::Orientation::Vertical, 0);
    editing.append(&scope);
    editing.append(&cells_b);
    editing.append(&scope_note);
    editing.append(&scroller);
    editing.append(&buttons);
    body.add_named(&editing, Some("editing"));
    body.set_visible_child_name("hint");

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // A fixed width, and nothing inside it allowed to argue: the document is the point of the
    // window, and a panel that negotiates for space takes two thirds of it.
    panel.set_size_request(SIDEBAR_WIDTH, -1);
    panel.set_hexpand(false);
    panel.append(&header);
    panel.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    panel.append(&body);
    panel.add_css_class("background");

    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    outer.set_hexpand(false);
    outer.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    outer.append(&panel);

    let root = gtk::Revealer::new();
    root.set_child(Some(&outer));
    root.set_hexpand(false);
    root.set_transition_type(gtk::RevealerTransitionType::SlideLeft);

    // --- moving values between the rows and the document ----------------------------------------
    let load_rows: Rc<dyn Fn()> = {
        let ui = ui.clone();
        let (font, radius, shadow, padding, margin, float, align) = (
            font.clone(), radius.clone(), shadow.clone(),
            padding.clone(), margin.clone(), float.clone(), align.clone(),
        );
        let width = width.clone();
        let (weight, slant, decoration) = (weight.clone(), slant.clone(), decoration.clone());
        let (letter_spacing, line_height) = (letter_spacing.clone(), line_height.clone());
        let (border_width, border_style, border_colour) =
            (border_width.clone(), border_style.clone(), border_colour.clone());
        let (text_colour, background) = (text_colour.clone(), background.clone());
        let font_touched = font_touched.clone();
        Rc::new(move || {
            let style = ui
                .key()
                .and_then(|k| ui.state.borrow().custom.get(&k).cloned())
                .unwrap_or_default();
            ui.loading.set(true);
            font_touched.set(!style.font_family.is_empty() || !style.font_size.is_empty());
            let family = if style.font_family.is_empty() {
                "DejaVu Sans".to_string()
            } else {
                style.font_family.trim_matches('"').to_string()
            };
            let size = style
                .font_size
                .trim_end_matches("pt")
                .trim_end_matches("px")
                .parse::<f64>()
                .unwrap_or(11.0);
            font.set_font_desc(&gtk::pango::FontDescription::from_string(&format!("{family} {size}")));
            weight.set_selected(WEIGHTS.iter().position(|w| *w == style.font_weight).unwrap_or(0) as u32);
            slant.set_selected(SLANTS.iter().position(|w| *w == style.font_style).unwrap_or(0) as u32);
            decoration.set_selected(
                DECORATIONS.iter().position(|d| *d == style.text_decoration).unwrap_or(0) as u32,
            );
            text_colour.set_hex(&style.colour);
            background.set_hex(&style.background);
            let (bw, bs, bc) = docstyle::parse_border(&style.border);
            border_width.set_value(bw as f64);
            border_style.set_selected(
                docstyle::BORDER_STYLES.iter().position(|s| *s == bs).unwrap_or(0) as u32,
            );
            if bw > 0 {
                border_colour.set_hex(&bc);
            } else {
                border_colour.set_hex("");
            }
            radius.set_value(style.radius.trim_end_matches("px").parse::<f64>().unwrap_or(0.0));
            shadow.set_active(!style.shadow.is_empty());
            set_length(&padding, &style.padding);
            set_length(&margin, &style.margin);
            set_length(&width, &style.width);
            set_length(&letter_spacing, &style.letter_spacing);
            line_height.set_value(style.line_height.parse::<f64>().unwrap_or(0.0));
            float.set_selected(FLOATS.iter().position(|f| *f == style.float).unwrap_or(0) as u32);
            align.set_selected(ALIGNS.iter().position(|a| *a == style.text_align).unwrap_or(0) as u32);
            ui.loading.set(false);
        })
    };

    let collect: Rc<dyn Fn() -> TagStyle> = {
        let (font, radius, shadow, padding, margin, float, align) = (
            font.clone(), radius.clone(), shadow.clone(),
            padding.clone(), margin.clone(), float.clone(), align.clone(),
        );
        let width = width.clone();
        let (weight, slant, decoration) = (weight.clone(), slant.clone(), decoration.clone());
        let (letter_spacing, line_height) = (letter_spacing.clone(), line_height.clone());
        let (border_width, border_style, border_colour) =
            (border_width.clone(), border_style.clone(), border_colour.clone());
        let (text_colour, background) = (text_colour.clone(), background.clone());
        let font_touched = font_touched.clone();
        Rc::new(move || {
            let desc = font.font_desc();
            let (family, size) = if font_touched.get() {
                (
                    desc.as_ref().and_then(|d| d.family()).map(|f| format!("\"{f}\"")).unwrap_or_default(),
                    desc.as_ref()
                        .filter(|d| d.size() > 0)
                        .map(|d| format!("{}pt", d.size() / gtk::pango::SCALE))
                        .unwrap_or_default(),
                )
            } else {
                // A font button always reports *something*. An element the panel merely displayed
                // must not grow a rule saying "the font you already had".
                (String::new(), String::new())
            };
            TagStyle {
                font_family: family,
                font_size: size,
                font_weight: pick(WEIGHTS, weight.selected()),
                font_style: pick(SLANTS, slant.selected()),
                text_decoration: pick(DECORATIONS, decoration.selected()),
                letter_spacing: get_length(&letter_spacing),
                line_height: if line_height.value() > 0.0 {
                    let v = format!("{:.2}", line_height.value());
                    v.trim_end_matches('0').trim_end_matches('.').to_string()
                } else {
                    String::new()
                },
                colour: text_colour.hex_or_empty(),
                background: background.hex_or_empty(),
                border: docstyle::compose_border(
                    border_width.value() as i64,
                    docstyle::BORDER_STYLES
                        .get(border_style.selected() as usize)
                        .copied()
                        .unwrap_or("solid"),
                    &{
                        let c = border_colour.hex_or_empty();
                        if c.is_empty() { "#000000".to_string() } else { c }
                    },
                ),
                radius: if radius.value() > 0.0 { format!("{}px", radius.value() as i64) } else { String::new() },
                shadow: if shadow.is_active() { docstyle::HOUSE_SHADOW.to_string() } else { String::new() },
                padding: get_length(&padding),
                margin: get_length(&margin),
                width: get_length(&width),
                float: FLOATS.get(float.selected() as usize).copied().filter(|f| *f != "—").unwrap_or("").to_string(),
                text_align: ALIGNS.get(align.selected() as usize).copied().filter(|a| *a != "—").unwrap_or("").to_string(),
            }
        })
    };

    // Reflect a freshly-resolved element into the whole panel.
    let show_selected: Rc<dyn Fn(Option<Selected>)> = {
        let ui = ui.clone();
        let (title, up, down, all_b, one_b, body, scope_note) =
            (title.clone(), up.clone(), down.clone(), all_b.clone(), one_b.clone(), body.clone(), scope_note.clone());
        let cells_b = cells_b.clone();
        let load_rows = load_rows.clone();
        Rc::new(move |selected: Option<Selected>| {
            let Some(s) = selected else {
                *ui.selected.borrow_mut() = None;
                title.set_text("<>");
                body.set_visible_child_name("hint");
                return;
            };
            title.set_text(&format!("<{}>", s.tag.to_uppercase()));
            up.set_sensitive(s.has_parent);
            down.set_sensitive(s.has_child);

            // "All instances" is only expressible for a tag the format can carry. Anything else —
            // a <main>, a <label> — can still be styled, but only as itself.
            // Containers of cells: styling one of these almost always means the cells.
            let container = matches!(s.tag.as_str(), "tr" | "thead" | "tbody" | "tfoot" | "table");
            cells_b.set_sensitive(container);
            if !container {
                cells_b.set_active(false);
            }

            let taggable = docstyle::STYLEABLE_TAGS.contains(&s.tag.as_str());
            all_b.set_sensitive(taggable);
            all_b.set_label(&format!("All <{}>", s.tag));

            // Piers' rule, revised 2026-08-06 UAT: the panel opens on THE SELECTED ELEMENT —
            // "this one" — every time. Document-wide is one click away on "All <tag>"; making
            // the wider blast radius the default was the wrong way round. (The 2026-08-01
            // default-All ruling is superseded by the author of both.)
            let instance = true;
            ui.instance_scope.set(instance);
            ui.loading.set(true);
            if instance {
                one_b.set_active(true);
            } else {
                all_b.set_active(true);
            }
            ui.loading.set(false);

            *ui.selected.borrow_mut() = Some(s);
            body.set_visible_child_name("editing");
            update_note(&scope_note, &ui);
            load_rows();
        })
    };

    // Ask the document what is selected now, and show it.
    let refresh_from: Rc<dyn Fn(String)> = {
        let ui = ui.clone();
        let show_selected = show_selected.clone();
        Rc::new(move |script: String| {
            let show_selected = show_selected.clone();
            ui.view.evaluate_javascript(
                &script,
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    let record = res.map(|v| v.to_str().to_string()).unwrap_or_default();
                    show_selected(Selected::parse(&record));
                },
            );
        })
    };

    {
        let refresh_from = refresh_from.clone();
        up.connect_clicked(move |_| refresh_from(docstyle::move_cursor_script(true)));
    }
    {
        let refresh_from = refresh_from.clone();
        down.connect_clicked(move |_| refresh_from(docstyle::move_cursor_script(false)));
    }

    // Switching scope re-reads the rows for the newly-targeted selector. Toggling back to "all"
    // therefore shows the page-wide values, which is what Apply will then keep.
    {
        let ui = ui.clone();
        let load_rows = load_rows.clone();
        let scope_note = scope_note.clone();
        one_b.connect_toggled(move |b| {
            if ui.loading.get() {
                return;
            }
            ui.instance_scope.set(b.is_active());
            update_note(&scope_note, &ui);
            load_rows();
        });
    }

    // --- Apply --------------------------------------------------------------------------------
    {
        let ui = ui.clone();
        let collect = collect.clone();
        let show_selected = show_selected.clone();
        let cells_apply = cells_b.clone();
        apply_b.connect_clicked(move |_| {
            let style = collect();
            let Some(selected) = ui.selected.borrow().clone() else { return };
            let before = ui.state.borrow().custom.clone();

            // "Apply to the cells inside" wins over the scope toggle: it is a more specific
            // statement of intent than "this one" vs "all of them".
            let to_cells = cells_apply.is_active() && cells_apply.is_sensitive();
            if ui.instance_scope.get() || to_cells {
                // This element alone — or every cell inside it, sharing one class.
                let minted = docstyle::next_instance_class(&before, selected.highest_instance);
                let ui2 = ui.clone();
                let show_selected = show_selected.clone();
                ui.view.evaluate_javascript(
                    &if to_cells {
                        docstyle::claim_cells_script(&minted)
                    } else {
                        docstyle::claim_instance_script(&minted)
                    },
                    None,
                    None,
                    gtk::gio::Cancellable::NONE,
                    move |res| {
                        let selector = res.map(|v| v.to_str().to_string()).unwrap_or_default();
                        if selector.is_empty() {
                            return;
                        }
                        let mut after = before.clone();
                        if style.is_empty() {
                            after.remove(&selector);
                        } else {
                            after.insert(selector.clone(), style.clone());
                        }
                        commit(&ui2, before.clone(), after, show_selected.clone());
                    },
                );
            } else {
                // Every element of this kind — and this element stops departing from it.
                let mut after = before.clone();
                if style.is_empty() {
                    after.remove(&selected.tag);
                } else {
                    after.insert(selected.tag.clone(), style);
                }
                // "on [Apply] the element specific CSS is deleted leaving the page wide CSS to
                // apply" — the override goes whether it was a minted class or the element's own id.
                //
                // The minted CLASS stays on the element, deliberately. Taking it off here would
                // make this un-undoable: nothing could find the element again to put it back. It is
                // inert without a rule, it is reused if the element is overridden again, and the
                // save path drops handles that no rule refers to, so it never reaches the file.
                if !selected.instance.is_empty() {
                    after.remove(&selected.instance);
                }
                commit(&ui, before, after, show_selected.clone());
            }
        });
    }

    // --- the handles the window drives it with ----------------------------------------------------
    let set_open: Rc<dyn Fn(bool)> = {
        let root = root.clone();
        let view = view.clone();
        Rc::new(move |open: bool| {
            root.set_reveal_child(open);
            if !open {
                // Take the dashed outline off, or a closed sidebar leaves a mark on the document.
                view.evaluate_javascript(
                    "window.wgCursor && window.wgCursor.clear()",
                    None,
                    None,
                    gtk::gio::Cancellable::NONE,
                    |_| {},
                );
            }
        })
    };

    {
        let set_open = set_open.clone();
        let show_selected = show_selected.clone();
        close.connect_clicked(move |_| {
            set_open(false);
            show_selected(None);
        });
    }

    let select_at: Rc<dyn Fn(f64, f64)> = {
        let root = root.clone();
        let refresh_from = refresh_from.clone();
        let view = view.clone();
        Rc::new(move |x: f64, y: f64| {
            if !root.reveals_child() {
                return;
            }
            // The widget's own size, so the script can scale widget pixels into CSS pixels.
            refresh_from(docstyle::select_at_script(x, y, view.width(), view.height()));
        })
    };

    let is_open: Rc<dyn Fn() -> bool> = {
        let root = root.clone();
        Rc::new(move || root.reveals_child())
    };

    let refresh: Rc<dyn Fn()> = {
        let root = root.clone();
        let refresh_from = refresh_from.clone();
        Rc::new(move || {
            if root.reveals_child() {
                refresh_from("window.wgCursor ? window.wgCursor.describe() : ''".to_string());
            }
        })
    };

    Sidebar { root, select_at, set_open, is_open, refresh }
}

/// A number-and-unit row, matching `stylerows::LengthRow`. Returned as its two widgets because the
/// sidebar's closures capture them separately.
fn length_row(
    title: &str,
    subtitle: &str,
    group: &adw::PreferencesGroup,
) -> (adw::SpinRow, gtk::DropDown) {
    let row = adw::SpinRow::with_range(0.0, 2000.0, 1.0);
    row.set_title(title);
    row.set_subtitle(subtitle);
    let unit = gtk::DropDown::from_strings(docstyle::LENGTH_UNITS);
    unit.set_valign(gtk::Align::Center);
    unit.set_tooltip_text(Some("Unit"));
    row.add_suffix(&unit);
    group.add(&row);
    (row, unit)
}

fn set_length(row: &(adw::SpinRow, gtk::DropDown), value: &str) {
    let (number, unit) = docstyle::parse_length(value);
    row.0.set_value(number);
    row.1.set_selected(docstyle::LENGTH_UNITS.iter().position(|u| *u == unit).unwrap_or(0) as u32);
}

fn get_length(row: &(adw::SpinRow, gtk::DropDown)) -> String {
    let unit = docstyle::LENGTH_UNITS.get(row.1.selected() as usize).copied().unwrap_or("px");
    docstyle::compose_length(row.0.value(), unit)
}

/// The chosen value from a combo whose first entry is the "say nothing" dash.
fn pick(options: &[&str], selected: u32) -> String {
    options.get(selected as usize).copied().filter(|v| *v != "—").unwrap_or("").to_string()
}

/// Put a new set of overrides into the document, remember how to take it back, and refresh the
/// panel from whatever the document says afterwards.
fn commit(
    ui: &Rc<Ui>,
    before: docstyle::CustomStyles,
    after: docstyle::CustomStyles,
    show_selected: Rc<dyn Fn(Option<Selected>)>,
) {
    if before == after {
        return;
    }
    ui.state.borrow_mut().custom = after.clone();
    write_block(ui);

    // The fingerprint is read *after* applying, which is the same value as before it: a style
    // change never moves it. It is what lets Undo tell a style change from a text edit.
    let ui2 = ui.clone();
    ui.view.evaluate_javascript(
        crate::undo::FINGERPRINT_JS,
        None,
        None,
        gtk::gio::Cancellable::NONE,
        move |res| {
            let fingerprint = res.map(|v| v.to_str().to_string()).unwrap_or_default();
            crate::undo::record(
                &ui2.state,
                crate::undo::StyleStep { before, after, fingerprint },
            );
            // The element may have just gained a handle, so re-read it.
            let show_selected = show_selected.clone();
            ui2.view.evaluate_javascript(
                "window.wgCursor.describe()",
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    let record = res.map(|v| v.to_str().to_string()).unwrap_or_default();
                    show_selected(Selected::parse(&record));
                },
            );
        },
    );
}

/// Say plainly which rule the next Apply will write, because the two scopes look identical.
fn update_note(note: &gtk::Label, ui: &Rc<Ui>) {
    let selected = ui.selected.borrow();
    let Some(s) = selected.as_ref() else { return };
    if ui.instance_scope.get() {
        note.set_text(&format!(
            "Only this <{}>, on top of the rule for all of them.",
            s.tag
        ));
    } else if s.instance.is_empty() {
        note.set_text(&format!("Every <{}> in the document.", s.tag));
    } else {
        note.set_text(&format!(
            "Every <{0}> in the document. Applying also drops this <{0}>'s own override.",
            s.tag
        ));
    }
}

/// Regenerate the document's style block from the map and put it in the live document.
fn write_block(ui: &Rc<Ui>) {
    let css = docstyle::custom_css(&ui.state.borrow().custom);
    docstyle::inject_custom(&ui.view, &css);
}

/// A colour row backed by the shared WebGen colour tool, so a colour chosen here is the same colour
/// available in Paint, Edit and Swatch, and the saved palettes are the same palettes.
///
/// It draws its own chip rather than using `swatch_button`, for one reason: the panel has to be able
/// to *set* the displayed colour when the selection changes. `swatch_button` owns its own drawing
/// and offers no way in, so moving from an element with red text to one with none would leave a red
/// chip on a row that is not setting anything.
#[derive(Clone)]
pub struct ColourRow {
    pub row: adw::ActionRow,
    chip: gtk::DrawingArea,
    value: Rc<Cell<webgen_swatch::Rgb>>,
    /// Whether this row is setting anything. An element the panel has not been used on must not
    /// grow a colour rule just because a picker had to show *some* colour.
    used: Rc<Cell<bool>>,
}

impl ColourRow {
    pub fn new(title: &str, settings: &Rc<Settings>, fallback: &str) -> ColourRow {
        let initial =
            webgen_swatch::Rgb::from_hex(fallback).unwrap_or(webgen_swatch::Rgb { r: 0, g: 0, b: 0 });
        let value = Rc::new(Cell::new(initial));
        let used = Rc::new(Cell::new(false));

        let chip = gtk::DrawingArea::new();
        chip.set_size_request(56, 22);
        {
            let (value, used) = (value.clone(), used.clone());
            chip.set_draw_func(move |_, cr, w, h| {
                webgen_swatch::paint_swatch(cr, w, h, value.get(), used.get(), false)
            });
        }

        let button = gtk::Button::new();
        button.set_child(Some(&chip));
        button.set_valign(gtk::Align::Center);
        button.set_tooltip_text(Some("Pick colour"));
        {
            let (value, used, chip) = (value.clone(), used.clone(), chip.clone());
            let reg = settings.swatch_reg();
            button.connect_clicked(move |btn| {
                let (value, used, chip) = (value.clone(), used.clone(), chip.clone());
                let popover = webgen_swatch::swatch_popover(&reg, btn, value.get(), move |rgb| {
                    value.set(rgb);
                    used.set(true);
                    chip.queue_draw();
                });
                popover.popup();
            });
        }

        let row = adw::ActionRow::new();
        row.set_title(title);
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));

        // Clearing a colour is a thing people need: it puts the element back on the rule beneath.
        let clear = gtk::Button::from_icon_name("edit-clear-symbolic");
        clear.set_valign(gtk::Align::Center);
        clear.add_css_class("flat");
        clear.set_tooltip_text(Some("Do not set a colour here"));
        {
            let (used, chip) = (used.clone(), chip.clone());
            clear.connect_clicked(move |_| {
                used.set(false);
                chip.queue_draw();
            });
        }
        row.add_suffix(&clear);

        ColourRow { row, chip, value, used }
    }

    pub fn set_hex(&self, hex: &str) {
        match webgen_swatch::Rgb::from_hex(hex) {
            Some(rgb) => {
                self.value.set(rgb);
                self.used.set(true);
            }
            None => self.used.set(false),
        }
        self.chip.queue_draw();
    }

    pub fn hex_or_empty(&self) -> String {
        if self.used.get() {
            self.value.get().to_hex()
        } else {
            String::new()
        }
    }
}
