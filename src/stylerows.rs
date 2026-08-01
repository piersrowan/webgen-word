//! The property rows that edit a [`TagStyle`], as one widget.
//!
//! There are two places in the app that let you style something — the document style sidebar and the
//! table window's CSS panel — and they must offer the same properties, read them the same way and
//! write them the same way, or a document styled in one will surprise you in the other. So they are
//! the same rows, built once here.
//!
//! Two behaviours are less obvious than they look and are the reason this is a struct rather than a
//! function that returns a box:
//!
//! - **A picker always reports *something*.** A font button asked for its description gives one
//!   whether or not anybody touched it, and a colour chip has to draw in some colour. Left alone,
//!   that would grow a rule on every element the panel merely *displayed*, saying "the font you
//!   already had". So each of those rows tracks whether it is actually setting anything, and
//!   [`StyleRows::collect`] leaves it out when it is not.
//! - **Loading values in is not the user making a choice**, so it is fenced with a flag.

use std::cell::Cell as CellFlag;
use std::rc::Rc;

use adw::prelude::*;

use crate::docstyle::{self, TagStyle};
use crate::settings::Settings;

pub const WEIGHTS: &[&str] = &["—", "normal", "bold"];
pub const SLANTS: &[&str] = &["—", "normal", "italic"];
pub const DECORATIONS: &[&str] = &["—", "none", "underline", "line-through"];
pub const FLOATS: &[&str] = &["—", "left", "right", "none"];
pub const ALIGNS: &[&str] = &["—", "left", "center", "right", "justify"];

/// The chosen value from a combo whose first entry is the "say nothing" dash.
pub fn pick(options: &[&str], selected: u32) -> String {
    options.get(selected as usize).copied().filter(|v| *v != "—").unwrap_or("").to_string()
}

fn index_of(options: &[&str], value: &str) -> u32 {
    options.iter().position(|o| *o == value).unwrap_or(0) as u32
}

pub struct StyleRows {
    /// Add this to a preferences page, or to any box.
    pub group: adw::PreferencesGroup,
    font: gtk::FontDialogButton,
    font_touched: Rc<CellFlag<bool>>,
    loading: Rc<CellFlag<bool>>,
    weight: adw::ComboRow,
    slant: adw::ComboRow,
    decoration: adw::ComboRow,
    text_colour: ColourRow,
    background: ColourRow,
    border_width: adw::SpinRow,
    border_style: adw::ComboRow,
    border_colour: ColourRow,
    radius: adw::SpinRow,
    shadow: adw::SwitchRow,
    padding: LengthRow,
    margin: LengthRow,
    width: LengthRow,
    float: adw::ComboRow,
    align: adw::ComboRow,
}

impl StyleRows {
    pub fn new(settings: &Rc<Settings>) -> StyleRows {
        let group = adw::PreferencesGroup::new();
        let loading = Rc::new(CellFlag::new(false));

        let font = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
        font.set_valign(gtk::Align::Center);
        let font_touched = Rc::new(CellFlag::new(false));
        {
            let (font_touched, loading) = (font_touched.clone(), loading.clone());
            font.connect_font_desc_notify(move |_| {
                if !loading.get() {
                    font_touched.set(true);
                }
            });
        }
        let font_row = adw::ActionRow::new();
        font_row.set_title("Font");
        font_row.add_suffix(&font);
        group.add(&font_row);

        // Bold / italic / underline. Combos rather than switches because "say nothing" and
        // "explicitly normal" are different answers: an explicit `normal` is how you take the bold
        // off something that inherits it, and a switch cannot express the difference.
        let weight = combo("Weight", WEIGHTS, &group);
        let slant = combo("Italic", SLANTS, &group);
        let decoration = combo("Underline", DECORATIONS, &group);

        let text_colour = ColourRow::new("Text colour", settings, "#1a1a1a");
        group.add(&text_colour.row);
        let background = ColourRow::new("Background", settings, "#ffffff");
        group.add(&background.row);

        // Border: three pickers, not a string to get wrong. **Width 0 removes the declaration
        // entirely** rather than writing `border: none`, which leaves the rule clean.
        let border_width = adw::SpinRow::with_range(0.0, 40.0, 1.0);
        border_width.set_title("Border width");
        border_width.set_subtitle("0 removes the border");
        group.add(&border_width);
        let border_style = combo("Border style", docstyle::BORDER_STYLES, &group);
        let border_colour = ColourRow::new("Border colour", settings, "#000000");
        group.add(&border_colour.row);

        let radius = adw::SpinRow::with_range(0.0, 60.0, 1.0);
        radius.set_title("Corner radius");
        group.add(&radius);

        let shadow = adw::SwitchRow::new();
        shadow.set_title("Drop shadow");
        group.add(&shadow);

        // Numbers with units, not free text. A typed `20` is not valid CSS and does nothing at
        // all, which is exactly what a text box invites; 0 clears the declaration, the same rule
        // as the border width above.
        let padding = LengthRow::new("Padding", "One value for all four sides", &group);
        let margin = LengthRow::new("Margin", "One value for all four sides", &group);
        let width = LengthRow::new("Width", "0 leaves the width to the content", &group);

        let float = combo("Float", FLOATS, &group);
        float.set_subtitle("Text wraps the other side");
        let align = combo("Alignment", ALIGNS, &group);

        StyleRows {
            group,
            font,
            font_touched,
            loading,
            weight,
            slant,
            decoration,
            text_colour,
            background,
            border_width,
            border_style,
            border_colour,
            radius,
            shadow,
            padding,
            margin,
            width,
            float,
            align,
        }
    }

    /// Show a style. Filling the rows in is not a choice, so nothing here counts as one.
    pub fn load(&self, style: &TagStyle) {
        self.loading.set(true);
        self.font_touched.set(!style.font_family.is_empty() || !style.font_size.is_empty());
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
        self.font.set_font_desc(&gtk::pango::FontDescription::from_string(&format!("{family} {size}")));
        self.weight.set_selected(index_of(WEIGHTS, &style.font_weight));
        self.slant.set_selected(index_of(SLANTS, &style.font_style));
        self.decoration.set_selected(index_of(DECORATIONS, &style.text_decoration));
        self.text_colour.set_hex(&style.colour);
        self.background.set_hex(&style.background);

        let (bw, bs, bc) = docstyle::parse_border(&style.border);
        self.border_width.set_value(bw as f64);
        self.border_style.set_selected(index_of(docstyle::BORDER_STYLES, &bs));
        self.border_colour.set_hex(if bw > 0 { &bc } else { "" });

        self.radius.set_value(style.radius.trim_end_matches("px").parse::<f64>().unwrap_or(0.0));
        self.shadow.set_active(!style.shadow.is_empty());
        self.padding.set(&style.padding);
        self.margin.set(&style.margin);
        self.width.set(&style.width);
        self.float.set_selected(index_of(FLOATS, &style.float));
        self.align.set_selected(index_of(ALIGNS, &style.text_align));
        self.loading.set(false);
    }

    /// Read the rows back. Anything nobody set stays unset.
    pub fn collect(&self) -> TagStyle {
        let desc = self.font.font_desc();
        let (family, size) = if self.font_touched.get() {
            (
                desc.as_ref().and_then(|d| d.family()).map(|f| format!("\"{f}\"")).unwrap_or_default(),
                desc.as_ref()
                    .filter(|d| d.size() > 0)
                    .map(|d| format!("{}pt", d.size() / gtk::pango::SCALE))
                    .unwrap_or_default(),
            )
        } else {
            (String::new(), String::new())
        };
        TagStyle {
            font_family: family,
            font_size: size,
            font_weight: pick(WEIGHTS, self.weight.selected()),
            font_style: pick(SLANTS, self.slant.selected()),
            text_decoration: pick(DECORATIONS, self.decoration.selected()),
            colour: self.text_colour.hex_or_empty(),
            background: self.background.hex_or_empty(),
            border: docstyle::compose_border(
                self.border_width.value() as i64,
                &pick_or(docstyle::BORDER_STYLES, self.border_style.selected(), "solid"),
                &{
                    let c = self.border_colour.hex_or_empty();
                    if c.is_empty() { "#000000".to_string() } else { c }
                },
            ),
            radius: if self.radius.value() > 0.0 {
                format!("{}px", self.radius.value() as i64)
            } else {
                String::new()
            },
            shadow: if self.shadow.is_active() {
                docstyle::HOUSE_SHADOW.to_string()
            } else {
                String::new()
            },
            padding: self.padding.get(),
            margin: self.margin.get(),
            width: self.width.get(),
            float: pick(FLOATS, self.float.selected()),
            text_align: pick(ALIGNS, self.align.selected()),
        }
    }
}

fn pick_or(options: &[&str], selected: u32, fallback: &str) -> String {
    options.get(selected as usize).copied().unwrap_or(fallback).to_string()
}

fn combo(title: &str, options: &[&str], group: &adw::PreferencesGroup) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.set_model(Some(&gtk::StringList::new(options)));
    group.add(&row);
    row
}

/// A CSS length: a number and a unit, never a free-text box.
///
/// The unit sits on the row as a dropdown rather than being typed, so a value cannot end up
/// unitless — `padding: 20` is not valid CSS, does nothing, and looks exactly like the setting
/// having been ignored. **0 clears the declaration**, the same rule as a border width.
#[derive(Clone)]
struct LengthRow {
    row: adw::SpinRow,
    unit: gtk::DropDown,
}

impl LengthRow {
    fn new(title: &str, subtitle: &str, group: &adw::PreferencesGroup) -> LengthRow {
        let row = adw::SpinRow::with_range(0.0, 2000.0, 1.0);
        row.set_title(title);
        row.set_subtitle(subtitle);
        let unit = gtk::DropDown::from_strings(docstyle::LENGTH_UNITS);
        unit.set_valign(gtk::Align::Center);
        unit.set_tooltip_text(Some("Unit"));
        row.add_suffix(&unit);
        group.add(&row);
        LengthRow { row, unit }
    }

    fn set(&self, value: &str) {
        let (number, unit) = docstyle::parse_length(value);
        self.row.set_value(number);
        self.unit
            .set_selected(docstyle::LENGTH_UNITS.iter().position(|u| *u == unit).unwrap_or(0) as u32);
    }

    fn get(&self) -> String {
        let unit = docstyle::LENGTH_UNITS
            .get(self.unit.selected() as usize)
            .copied()
            .unwrap_or("px");
        docstyle::compose_length(self.row.value(), unit)
    }
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
    value: Rc<CellFlag<webgen_swatch::Rgb>>,
    /// Whether this row is setting anything.
    used: Rc<CellFlag<bool>>,
}

impl ColourRow {
    pub fn new(title: &str, settings: &Rc<Settings>, fallback: &str) -> ColourRow {
        let initial =
            webgen_swatch::Rgb::from_hex(fallback).unwrap_or(webgen_swatch::Rgb { r: 0, g: 0, b: 0 });
        let value = Rc::new(CellFlag::new(initial));
        let used = Rc::new(CellFlag::new(false));

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
