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

    let scope_note = gtk::Label::new(None);
    scope_note.add_css_class("dim-label");
    scope_note.add_css_class("caption");
    scope_note.set_wrap(true);
    scope_note.set_xalign(0.0);
    scope_note.set_margin_start(10);
    scope_note.set_margin_end(10);
    scope_note.set_margin_top(4);

    // --- the properties --------------------------------------------------------------------------
    let group = adw::PreferencesGroup::new();

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

    let text_colour = ColourRow::new("Text colour", settings, "#1a1a1a");
    group.add(&text_colour.row);
    let background = ColourRow::new("Background", settings, "#ffffff");
    group.add(&background.row);

    let border = adw::EntryRow::new();
    border.set_title("Border");
    border.set_tooltip_text(Some("A CSS border, e.g. 1px solid red"));
    group.add(&border);

    let radius = adw::SpinRow::with_range(0.0, 60.0, 1.0);
    radius.set_title("Corner radius");
    group.add(&radius);

    let shadow = adw::SwitchRow::new();
    shadow.set_title("Drop shadow");
    group.add(&shadow);

    let padding = adw::EntryRow::new();
    padding.set_title("Padding");
    padding.set_tooltip_text(Some("One value for all four sides, e.g. 12px"));
    group.add(&padding);

    let margin = adw::EntryRow::new();
    margin.set_title("Margin");
    margin.set_tooltip_text(Some("One value for all four sides, e.g. 6px"));
    group.add(&margin);

    let float = adw::ComboRow::new();
    float.set_title("Float");
    float.set_subtitle("Text wraps the other side");
    float.set_model(Some(&gtk::StringList::new(FLOATS)));
    group.add(&float);

    let align = adw::ComboRow::new();
    align.set_title("Alignment");
    align.set_model(Some(&gtk::StringList::new(ALIGNS)));
    group.add(&align);

    let rows_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    rows_box.set_margin_top(12);
    rows_box.set_margin_start(8);
    rows_box.set_margin_end(8);
    rows_box.append(&group);
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
        let (font, border, radius, shadow, padding, margin, float, align) = (
            font.clone(), border.clone(), radius.clone(), shadow.clone(),
            padding.clone(), margin.clone(), float.clone(), align.clone(),
        );
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
            text_colour.set_hex(&style.colour);
            background.set_hex(&style.background);
            border.set_text(&style.border);
            radius.set_value(style.radius.trim_end_matches("px").parse::<f64>().unwrap_or(0.0));
            shadow.set_active(!style.shadow.is_empty());
            padding.set_text(&style.padding);
            margin.set_text(&style.margin);
            float.set_selected(FLOATS.iter().position(|f| *f == style.float).unwrap_or(0) as u32);
            align.set_selected(ALIGNS.iter().position(|a| *a == style.text_align).unwrap_or(0) as u32);
            ui.loading.set(false);
        })
    };

    let collect: Rc<dyn Fn() -> TagStyle> = {
        let (font, border, radius, shadow, padding, margin, float, align) = (
            font.clone(), border.clone(), radius.clone(), shadow.clone(),
            padding.clone(), margin.clone(), float.clone(), align.clone(),
        );
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
                colour: text_colour.hex_or_empty(),
                background: background.hex_or_empty(),
                border: border.text().trim().to_string(),
                radius: if radius.value() > 0.0 { format!("{}px", radius.value() as i64) } else { String::new() },
                shadow: if shadow.is_active() { docstyle::HOUSE_SHADOW.to_string() } else { String::new() },
                padding: padding.text().trim().to_string(),
                margin: margin.text().trim().to_string(),
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
            let taggable = docstyle::STYLEABLE_TAGS.contains(&s.tag.as_str());
            all_b.set_sensitive(taggable);
            all_b.set_label(&format!("All <{}>", s.tag));

            // Piers' rule: an element the document already styles specifically opens on "this one".
            let instance = s.specific || !taggable;
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
        apply_b.connect_clicked(move |_| {
            let style = collect();
            let Some(selected) = ui.selected.borrow().clone() else { return };

            if ui.instance_scope.get() {
                // This element alone. Mint it a handle if it has none, then write the override.
                let minted = docstyle::next_instance_class(
                    &ui.state.borrow().custom,
                    selected.highest_instance,
                );
                let ui2 = ui.clone();
                let show_selected = show_selected.clone();
                ui.view.evaluate_javascript(
                    &docstyle::claim_instance_script(&minted),
                    None,
                    None,
                    gtk::gio::Cancellable::NONE,
                    move |res| {
                        let selector = res.map(|v| v.to_str().to_string()).unwrap_or_default();
                        if selector.is_empty() {
                            return;
                        }
                        {
                            let mut st = ui2.state.borrow_mut();
                            if style.is_empty() {
                                st.custom.remove(&selector);
                            } else {
                                st.custom.insert(selector.clone(), style.clone());
                            }
                        }
                        write_block(&ui2);
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
            } else {
                // Every element of this kind — and this element stops departing from it.
                {
                    let mut st = ui.state.borrow_mut();
                    if style.is_empty() {
                        st.custom.remove(&selected.tag);
                    } else {
                        st.custom.insert(selected.tag.clone(), style);
                    }
                    // "on [Apply] the element specific CSS is deleted leaving the page wide CSS to
                    // apply" — the override goes whether it was a minted class or the element's own
                    // id, so what is on screen is now the page-wide rule and nothing else.
                    if !selected.instance.is_empty() {
                        st.custom.remove(&selected.instance);
                    }
                }
                write_block(&ui);
                // Take the minted class back off the element so dead handles do not accumulate.
                let show_selected = show_selected.clone();
                ui.view.evaluate_javascript(
                    &docstyle::release_instance_script(),
                    None,
                    None,
                    gtk::gio::Cancellable::NONE,
                    move |res| {
                        let record = res.map(|v| v.to_str().to_string()).unwrap_or_default();
                        show_selected(Selected::parse(&record));
                    },
                );
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
        Rc::new(move |x: f64, y: f64| {
            if !root.reveals_child() {
                return;
            }
            refresh_from(docstyle::select_at_script(x, y));
        })
    };

    let is_open: Rc<dyn Fn() -> bool> = {
        let root = root.clone();
        Rc::new(move || root.reveals_child())
    };

    Sidebar { root, select_at, set_open, is_open }
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
