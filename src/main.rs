//! **webgen-word** — a light word processor whose document format is plain HTML.
//!
//! The premise: a CV, a letter or a report is a *document*, and HTML+CSS already describes documents
//! perfectly well. So the file on disk is a standalone `.html` that any browser opens, and "export
//! to PDF" is just printing. No proprietary container, nothing to convert, nothing to lose.
//!
//! Editing is `WebView::set_editable` — WebKit supplies the caret, selection, undo, clipboard and
//! IME behaviour, and the toolbar drives it through `execute_editing_command`. Reimplementing that
//! over a text buffer would be months of work and worse at the end of it.
//!
//! ## A word processor, not an HTML editor
//!
//! That distinction decides most of the design. A document here holds text, structure, pictures and
//! style — and nothing else. Script, embedded objects and form controls are cut out of every
//! document on the way in ([`sanitise`], which runs on the source **before** WebKit ever parses it)
//! and again on the way out. Pictures are embedded as `data:` URIs so the file stays one file. The
//! only references a saved document may make to the outside world are stylesheets and fonts.
//!
//! ## Style: base plus per-document
//!
//! The house style — font, colours, page — lives in the shared registry and is edited in System
//! Settings from the manifest this repo ships (CONTRACT.md §2/§3). A particular document's
//! departures from it live *in the document*, in a second `<style>` block written in a fixed layout
//! so it reads back exactly as it was written. See [`docstyle`]; the format is shared with the
//! browser's editor.
//!
//! ## The one thing that is not obvious
//!
//! **CSS `@page` does nothing on our print path.** Measured before this app was written: a document
//! asking for A4 with 45mm/40mm margins printed as US Letter with 10mm/7mm — GTK's defaults.
//! WebKit's GTK print backend takes page geometry from `gtk::PageSetup` and ignores the stylesheet
//! entirely. That is why [`page`] exists and why Page setup is a menu item rather than a CSS comment.

mod assets;
mod doc;
mod docstyle;
mod js;
mod page;
mod paged;
mod paginate;
mod sanitise;
mod settings;
mod sidebar;
mod stylerows;
mod table;
mod table_window;
mod undo;

use adw::prelude::*;
use docstyle::{Base, CustomStyles};
use gtk::glib;
use page::{PageSetup, Paper};
use settings::Settings;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use webkit6::prelude::*;

const APP_ID: &str = settings::APP_ID;

/// Everything the window needs to know about the open document.
pub struct State {
    /// Where it came from / goes back to. `None` until first save.
    pub path: Option<PathBuf>,
    /// This document's page geometry — its own, read from its `<meta name="webgen-page">`, not the
    /// app's. Until 0.3.0 it was the app's, so opening a CV authored at A5 printed it at A4.
    pub setup: PageSetup,
    /// The document's HTML as it stood at the last load or save, read back OUT OF THE DOM rather
    /// than off disk. WebKit normalises markup on parse, so the file's own bytes are not a usable
    /// baseline -- comparing against them would report "modified" on a document nobody touched.
    /// This is what makes "close without saving?" ask only when there is something to lose.
    pub baseline: String,
    /// This document's style overrides, as last read from or written to it.
    pub custom: CustomStyles,
    /// A `.docx` conversion's temporary home (Piers, 2026-08-06: looking at a document must not
    /// litter). Set while the document on screen lives in a convert dir under the cache; deleted
    /// whole the moment that document is closed, replaced, or SAVED to a real home.
    pub temp_convert: Option<PathBuf>,
    /// Where a converted document would naturally live: `stem.html` beside the original `.docx`.
    /// Save As offers this; nothing is written there until the user says so.
    pub suggested: Option<PathBuf>,
    /// Style changes that can be taken back, oldest first. Interleaved with WebKit's own text
    /// history by [`undo`], so Undo always reverses the last action whichever kind it was.
    pub undo: Vec<undo::StyleStep>,
    pub redo: Vec<undo::StyleStep>,
}

/// What goes in the title bar: the FILE NAME, not the document's `<title>`.
///
/// It used to be `<title>`, which meant a new document said "Untitled" forever -- saving it as
/// `cv.html` changed nothing on screen, because the heading inside the file still said Untitled.
/// Every word processor titles the window after the file; that is the thing you are looking for
/// when you scan a taskbar.
fn doc_title(path: &Option<PathBuf>) -> String {
    match path {
        Some(p) => p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into()),
        None => "Untitled".into(),
    }
}

fn main() -> glib::ExitCode {
    // A crash cannot clean its own convert dir, so each launch sweeps ancient ones.
    sweep_stale_converts();
    let app = adw::Application::builder()
        .application_id(APP_ID)
        // Positional file arguments, per CONTRACT.md 5 -- `webgen-word cv.html` and "Open With"
        // must both work, and a second invocation opens in the running instance.
        .flags(gtk::gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    let settings = Rc::new(Settings::open());
    {
        let settings = settings.clone();
        app.connect_activate(move |a| build(a, &settings, None));
    }
    {
        let settings = settings.clone();
        app.connect_open(move |a, files, _| {
            for f in files {
                build(a, &settings, f.path());
            }
        });
    }
    app.run()
}

/// A toolbar button: icon, tooltip, and nothing else. Every control here is icon-only with a
/// tooltip, because a word-processor toolbar with twelve text labels is unreadable.
fn tool(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.add_css_class("flat");
    b
}

/// Register a window action and return its fully-qualified name, for menus and key bindings.
fn action<F: Fn() + 'static>(window: &adw::ApplicationWindow, name: &str, f: F) -> String {
    let a = gtk::gio::SimpleAction::new(name, None);
    a.connect_activate(move |_, _| f());
    window.add_action(&a);
    format!("win.{name}")
}

fn build(app: &adw::Application, settings: &Rc<Settings>, open: Option<PathBuf>) {
    let state = Rc::new(RefCell::new(State {
        path: open.clone(),
        setup: PageSetup::from_settings(settings),
        baseline: String::new(),
        custom: CustomStyles::new(),
        temp_convert: None,
        suggested: None,
        undo: Vec::new(),
        redo: Vec::new(),
    }));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Word")
        .default_width(980)
        .default_height(800)
        .build();

    let view = webkit6::WebView::new();
    view.set_editable(true);
    view.set_vexpand(true);
    // Belt and braces on top of the sanitiser. The text pass in `sanitise` is what actually keeps
    // script out — it runs before WebKit sees the markup — but turning off script *in markup* costs
    // nothing and closes anything the scanner might not have understood. `enable_javascript_markup`
    // rather than `enable_javascript`: the app's own `evaluate_javascript` is how saving, styling
    // and page setup all work, and disabling JavaScript outright would break every one of them.
    if let Some(s) = webkit6::prelude::WebViewExt::settings(&view) {
        s.set_enable_javascript_markup(false);
        s.set_enable_html5_database(false);
        s.set_enable_html5_local_storage(false);
    }

    // A place to say what happened — what the sanitiser removed, or why a save failed. Silence was
    // the old behaviour and it is the wrong one: a document that quietly loses part of itself, or a
    // save that quietly does not happen, are exactly the things a person must be told about.
    let banner = adw::Banner::new("");
    banner.set_button_label(Some("Dismiss"));
    {
        let banner = banner.clone();
        banner.clone().connect_button_clicked(move |_| banner.set_revealed(false));
    }
    let say = {
        let banner = banner.clone();
        Rc::new(move |text: &str| {
            banner.set_title(text);
            banner.set_revealed(true);
        })
    };

    // --- header ------------------------------------------------------------------------------
    let header = adw::HeaderBar::new();

    let new_b = tool("document-new-symbolic", "New document  (Ctrl+N)");
    let open_b = tool("document-open-symbolic", "Open…  (Ctrl+O)");
    let save_b = tool("document-save-symbolic", "Save  (Ctrl+S)");
    header.pack_start(&new_b);
    header.pack_start(&open_b);
    header.pack_start(&save_b);

    let print_b = tool("document-print-symbolic", "Print / export PDF  (Ctrl+P)");
    let style_b = gtk::ToggleButton::new();
    style_b.set_icon_name("applications-graphics-symbolic");
    style_b.set_tooltip_text(Some("Style sidebar — click anything in the document to style it"));
    style_b.add_css_class("flat");
    let menu_b = gtk::MenuButton::new();
    menu_b.set_icon_name("open-menu-symbolic");
    menu_b.set_tooltip_text(Some("Menu"));
    header.pack_end(&menu_b);
    header.pack_end(&print_b);
    header.pack_end(&style_b);

    // Pages view (read-only two-page spread; see `paged`). The toggle lives in the header; the
    // [<] Pages x–y of N [>] strip only exists while the view is on.
    let pages_b = gtk::ToggleButton::builder()
        .icon_name("view-dual-symbolic")
        .tooltip_text("Pages view — read the document as spreads (Esc leaves)")
        .build();
    let pages_prev = gtk::Button::from_icon_name("go-previous-symbolic");
    let pages_next = gtk::Button::from_icon_name("go-next-symbolic");
    let pages_label = gtk::Label::new(None);
    let pages_nav = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    pages_nav.append(&pages_prev);
    pages_nav.append(&pages_label);
    pages_nav.append(&pages_next);
    pages_nav.set_visible(false);
    header.pack_end(&pages_nav);
    header.pack_end(&pages_b);

    // Shared view-mode state, created early so the save/print/timer guards further down can see
    // it. `spread_first` is the LEFT page of the visible pair, 0-based.
    let paged_active: Rc<std::cell::Cell<bool>> = Rc::new(std::cell::Cell::new(false));
    let spread_first: Rc<std::cell::Cell<usize>> = Rc::new(std::cell::Cell::new(0));
    let page_total: Rc<std::cell::Cell<usize>> = Rc::new(std::cell::Cell::new(1));

    {
        // Entering wraps the document into sheets and drops editability; leaving unwraps and
        // restores it. The show step is shared with [<] [>].
        let show_spread = {
            let view = view.clone();
            let label = pages_label.clone();
            let spread_first = spread_first.clone();
            let page_total = page_total.clone();
            Rc::new(move || {
                let first = spread_first.get();
                let total = page_total.get();
                let last_shown = (first + 2).min(total);
                label.set_text(&format!("Pages {}–{} of {}", first + 1, last_shown, total));
                view.evaluate_javascript(
                    &paged::show_script(first),
                    None,
                    None,
                    gtk::gio::Cancellable::NONE,
                    |_| {},
                );
            })
        };
        {
            let view = view.clone();
            let state = state.clone();
            let nav = pages_nav.clone();
            let paged_active = paged_active.clone();
            let spread_first = spread_first.clone();
            let page_total = page_total.clone();
            let show_spread = show_spread.clone();
            pages_b.connect_toggled(move |b| {
                if b.is_active() {
                    let px = paginate::inner_height_px(&state.borrow().setup);
                    let view2 = view.clone();
                    let nav = nav.clone();
                    let paged_active = paged_active.clone();
                    let spread_first = spread_first.clone();
                    let page_total = page_total.clone();
                    let show_spread = show_spread.clone();
                    view.evaluate_javascript(
                        &paged::enter_script(px),
                        None,
                        None,
                        gtk::gio::Cancellable::NONE,
                        move |res| {
                            let pages: usize = res
                                .ok()
                                .map(|v| v.to_str().to_string())
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0);
                            if pages == 0 {
                                return; // nothing wrapped (empty document); stay in edit view
                            }
                            paged_active.set(true);
                            page_total.set(pages);
                            spread_first.set(0);
                            view2.set_editable(false);
                            nav.set_visible(true);
                            show_spread();
                        },
                    );
                } else if paged_active.get() {
                    paged_active.set(false);
                    nav.set_visible(false);
                    view.set_editable(true);
                    view.evaluate_javascript(
                        &paged::exit_script(),
                        None,
                        None,
                        gtk::gio::Cancellable::NONE,
                        |_| {},
                    );
                }
            });
        }
        {
            let spread_first = spread_first.clone();
            let show_spread = show_spread.clone();
            pages_prev.connect_clicked(move |_| {
                let f = spread_first.get();
                if f > 0 {
                    spread_first.set(f - 1);
                    show_spread();
                }
            });
        }
        {
            let spread_first = spread_first.clone();
            let page_total = page_total.clone();
            pages_next.connect_clicked(move |_| {
                // The last spread ends ON the last page (…, N-1,N) — never walks past it.
                let f = spread_first.get();
                if f < page_total.get().saturating_sub(2) {
                    spread_first.set(f + 1);
                    show_spread();
                }
            });
        }
    }

    // --- formatting row ----------------------------------------------------------------------
    // `execute_editing_command` names are WebKit's own.
    let fmt = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    fmt.set_margin_start(8);
    fmt.set_margin_end(8);
    fmt.set_margin_top(4);
    fmt.set_margin_bottom(4);

    let style = gtk::DropDown::from_strings(&["Body", "Title", "Heading", "Subheading"]);
    style.set_tooltip_text(Some("Paragraph style"));
    style.set_valign(gtk::Align::Center);
    fmt.append(&style);
    fmt.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    for (icon, tip, cmd) in [
        ("format-text-bold-symbolic", "Bold", "Bold"),
        ("format-text-italic-symbolic", "Italic", "Italic"),
        ("format-text-underline-symbolic", "Underline", "Underline"),
    ] {
        let b = tool(icon, tip);
        let view = view.clone();
        b.connect_clicked(move |_| view.execute_editing_command(cmd));
        fmt.append(&b);
    }
    fmt.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    for (icon, tip, cmd) in [
        ("view-list-symbolic", "Bulleted list", "InsertUnorderedList"),
        ("view-list-ordered-symbolic", "Numbered list", "InsertOrderedList"),
    ] {
        let b = tool(icon, tip);
        let view = view.clone();
        b.connect_clicked(move |_| view.execute_editing_command(cmd));
        fmt.append(&b);
    }
    // Indent / outdent. In a list these are how you make and unmake SUB-LISTS, which is the only
    // way to nest one -- there is no other gesture for it besides Tab, bound below.
    for (icon, tip, cmd) in [
        ("format-indent-less-symbolic", "Decrease indent  (Shift+Tab)", "Outdent"),
        ("format-indent-more-symbolic", "Increase indent — makes a sub-list  (Tab)", "Indent"),
    ] {
        let b = tool(icon, tip);
        let view = view.clone();
        b.connect_clicked(move |_| view.execute_editing_command(cmd));
        fmt.append(&b);
    }
    fmt.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    for (icon, tip, cmd) in [
        ("format-justify-left-symbolic", "Align left", "JustifyLeft"),
        ("format-justify-center-symbolic", "Centre", "JustifyCenter"),
        ("format-justify-right-symbolic", "Align right", "JustifyRight"),
        ("format-justify-fill-symbolic", "Justify", "JustifyFull"),
    ] {
        let b = tool(icon, tip);
        let view = view.clone();
        b.connect_clicked(move |_| view.execute_editing_command(cmd));
        fmt.append(&b);
    }
    fmt.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    let image_b = tool("insert-image-symbolic", "Insert picture — embedded in the document");
    fmt.append(&image_b);

    let table_b = tool("view-grid-symbolic", "Table — insert one here, or edit the one the cursor is in");
    fmt.append(&table_b);

    let link_b = tool("insert-link-symbolic", "Link…  (Ctrl+K)");
    fmt.append(&link_b);

    // A page break is the one thing a CV genuinely needs and HTML has no key for. It goes in as a
    // styled `<hr>`; the stylesheet turns it into `break-before: page` and draws it while editing.
    let brk = tool("go-bottom-symbolic", "Insert page break  (Ctrl+Return)");
    {
        let view = view.clone();
        brk.connect_clicked(move |_| {
            view.execute_editing_command_with_argument(
                "InsertHTML",
                &format!("<hr class=\"{}\">", docstyle::PAGE_BREAK_CLASS),
            );
        });
    }
    fmt.append(&brk);

    let clear = tool("edit-clear-symbolic", "Remove formatting");
    {
        let view = view.clone();
        clear.connect_clicked(move |_| view.execute_editing_command("RemoveFormat"));
    }
    fmt.append(&clear);
    fmt.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    // Undo and Redo go through `undo`, not straight to WebKit: a style change is an action too,
    // and pressing Undo after one must take THAT back rather than the last thing that was typed.
    let undo_b = tool("edit-undo-symbolic", "Undo  (Ctrl+Z)");
    let redo_b = tool("edit-redo-symbolic", "Redo  (Ctrl+Shift+Z)");
    fmt.append(&undo_b);
    fmt.append(&redo_b);

    {
        let view = view.clone();
        style.connect_selected_notify(move |d| {
            let tag = match d.selected() {
                1 => "h1",
                2 => "h2",
                3 => "h3",
                _ => "p",
            };
            view.evaluate_javascript(
                &format!("document.execCommand('formatBlock',false,{})", js::string(tag)),
                None,
                None,
                gtk::gio::Cancellable::NONE,
                |_| {},
            );
        });
    }

    // --- document load -------------------------------------------------------------------------
    // A slot holding load_into itself, so the .docx convert dialog's callback can re-enter the
    // load with the html it produced. Filled right after the closure exists.
    let load_self: Rc<RefCell<Option<Rc<dyn Fn(Option<PathBuf>)>>>> = Rc::new(RefCell::new(None));
    let load_into: Rc<dyn Fn(Option<PathBuf>)> = {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        let settings = settings.clone();
        let say = say.clone();
        let load_self = load_self.clone();
        Rc::new(move |path: Option<PathBuf>| {
            let base = Base::from_settings(&settings);
            // A `.wgz` is unpacked beside itself and the document inside is what actually opens.
            // Unpacking rather than working inside the archive is deliberate: the working form is
            // markup plus a folder, and a format you can only edit through a zip is the thing this
            // app exists not to be.
            let path = match path {
                Some(p) if is_wgz(&p) => match unpack_wgz(&p) {
                    Ok((doc, where_to)) => {
                        say(&format!("Unpacked into {} — editing the document there.", where_to.display()));
                        Some(doc)
                    }
                    Err(e) => {
                        say(&format!("Could not open {}: {e}", p.display()));
                        None
                    }
                },
                // A .docx converts to html-plus-folder beside itself and THAT opens — same deal
                // as the .wgz above. The original .docx is never modified. HOW its styling comes
                // across is the user's call (Piers's three modes), so a dialog asks first and the
                // conversion happens in its response; this invocation ends here.
                Some(p) if is_docx(&p) => {
                    let dlg = adw::MessageDialog::new(
                        Some(&window),
                        Some("Convert Word document"),
                        Some("How should its styling come across?"),
                    );
                    // NB stacked responses render in reverse order (see confirm_if_modified).
                    dlg.add_response("cancel", "Cancel");
                    dlg.add_response("plain", "Plain");
                    dlg.add_response("system", "System style");
                    dlg.add_response("document", "Document formatting");
                    let last = settings.string("docx_convert_mode", "document");
                    dlg.set_default_response(Some(&last));
                    dlg.set_close_response("cancel");
                    let settings = settings.clone();
                    let say = say.clone();
                    let again = load_self.clone();
                    let state = state.clone();
                    dlg.connect_response(None, move |dlg, resp| {
                        dlg.close();
                        if resp == "cancel" {
                            return;
                        }
                        let mode = ConvertMode::from_key(resp);
                        settings.set_string("docx_convert_mode", mode.key());
                        match convert_docx(&p, mode) {
                            Ok((doc, tmp_dir, suggested, pictures, setup)) => {
                                if let Some(setup) = setup {
                                    state.borrow_mut().setup = setup;
                                }
                                let extra = if pictures > 0 {
                                    format!(" (+{pictures} picture{})", if pictures == 1 { "" } else { "s" })
                                } else {
                                    String::new()
                                };
                                say(&format!(
                                    "Converted{extra} — viewing a copy. Save to keep it (suggests {}).",
                                    suggested.display()
                                ));
                                // Any previous conversion this window still owned dies first —
                                // overwriting the field would leak its dir until the sweep.
                                drop_temp_convert(&state, None);
                                {
                                    let mut s = state.borrow_mut();
                                    s.temp_convert = Some(tmp_dir);
                                    s.suggested = Some(suggested);
                                }
                                if let Some(f) = again.borrow().as_ref() {
                                    f(Some(doc));
                                }
                            }
                            Err(e) => say(&format!("Could not open {}: {e}", p.display())),
                        }
                    });
                    dlg.present();
                    return;
                }
                other => other,
            };
            // The document being replaced may have been a docx conversion living in a temp dir —
            // if what loads next is not from that same dir, the dir and everything in it goes.
            // (The docx arm above returned early, so a CANCELLED convert dialog never lands here
            // and cannot delete the document still on screen.)
            drop_temp_convert(&state, path.as_deref());
            let bytes = path.as_ref().map(|p| (p.clone(), std::fs::read(p)));

            let (html, setup, path, report) = match bytes {
                // Opened a file that reads and looks like a document.
                Some((p, Ok(raw))) if doc::looks_like_html(&raw) => {
                    let source = String::from_utf8_lossy(&raw).to_string();
                    // Cut script and imported assets out of the SOURCE. Doing this after loading
                    // would be doing it after the script had already run.
                    // Opening does not move anybody's files about; that happens on save.
                    let (clean, report) =
                        sanitise::clean(&source, p.parent(), &sanitise::AssetPolicy::Keep);
                    // The document's own geometry wins over the app's default.
                    let setup = doc::page_setup_of(&clean).unwrap_or_else(|| PageSetup::from_settings(&settings));
                    (doc::prepare(&clean, &docstyle::base_css(&base, setup), setup), setup, Some(p), report)
                }
                // It read, but it is not a document. Say so rather than silently blanking it.
                Some((p, Ok(_))) => {
                    say(&format!(
                        "“{}” does not look like an HTML document, so it was not opened.",
                        p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                    ));
                    let setup = PageSetup::from_settings(&settings);
                    (doc::blank(&base, setup), setup, None, sanitise::Report::default())
                }
                // It would not read at all.
                Some((p, Err(e))) => {
                    say(&format!("Could not open {}: {e}", p.display()));
                    let setup = PageSetup::from_settings(&settings);
                    (doc::blank(&base, setup), setup, None, sanitise::Report::default())
                }
                // A new document.
                None => {
                    let setup = PageSetup::from_settings(&settings);
                    (doc::blank(&base, setup), setup, None, sanitise::Report::default())
                }
            };

            // base URI = the file's own directory, so anything still relative resolves.
            let base_uri = path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|d| format!("file://{}/", d.display()));
            view.load_html(&html, base_uri.as_deref());
            window.set_title(Some(&format!("{} — Word", doc_title(&path))));

            {
                let mut s = state.borrow_mut();
                s.path = path;
                s.setup = setup;
                s.custom = docstyle::parse_custom_css(&doc::custom_block(&html));
                // A different document is a different history.
                s.undo.clear();
                s.redo.clear();
            }
            if let Some(summary) = report.summary() {
                say(&summary);
            }
        })
    };
    *load_self.borrow_mut() = Some(load_into.clone());

    // Re-baseline whenever a document finishes loading. Registered BEFORE the first load so the
    // initial document is baselined too -- `load_html` is asynchronous, the signal arrives later.
    {
        let state = state.clone();
        view.connect_load_changed(move |v, ev| {
            if ev != webkit6::LoadEvent::Finished {
                return;
            }
            // The Picture menu needs to know which picture was clicked. WebKit's HitTestResult says
            // "an image" but not *which*, so the page marks the one the menu was opened on. This is
            // the app's own script, injected here — not script from the document, which is gone.
            // The style sidebar's DOM half: helpers that remember and move the cursor.
            v.evaluate_javascript(
                &docstyle::cursor_script(),
                None,
                None,
                gtk::gio::Cancellable::NONE,
                |_| {},
            );
            v.evaluate_javascript(
                &format!(
                    "document.addEventListener('contextmenu', function (e) {{
                       document.querySelectorAll('.wg-selected').forEach(function (el) {{
                         el.classList.remove('wg-selected');
                       }});
                       if (e.target && e.target.tagName === 'IMG') {{
                         e.target.classList.add({selected});
                       }}
                     }}, true)",
                    selected = js::string("wg-selected")
                ),
                None,
                None,
                gtk::gio::Cancellable::NONE,
                |_| {},
            );

            let state = state.clone();
            v.evaluate_javascript(
                "document.documentElement.outerHTML",
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    if let Ok(val) = res {
                        state.borrow_mut().baseline = val.to_str().to_string();
                    }
                },
            );
        });
    }

    load_into(open);

    // --- save ------------------------------------------------------------------------------------
    // The document is read back out of the live DOM, so what saves is exactly what is on screen.
    // `single_file` is the export form: everything inlined as `data:` URIs, nothing beside the
    // document. The normal save puts pictures in `<stem>_files/` next to it.
    let save_to = {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        let say = say.clone();
        Rc::new(move |path: PathBuf, single_file: bool| {
            let window = window.clone();
            let state = state.clone();
            let say = say.clone();
            let title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let page_meta = state.borrow().setup.to_meta();
            // Where the document's references resolve *today*. Saving somewhere else must copy the
            // pictures across, and that falls out of resolving against the old home while writing
            // the folder beside the new one.
            let source_dir = state
                .borrow()
                .path
                .as_ref()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .or_else(|| path.parent().map(|d| d.to_path_buf()));
            let folder = assets::folder_name(&path);
            let policy = if single_file {
                sanitise::AssetPolicy::Embed
            } else {
                sanitise::AssetPolicy::Folder {
                    dir: assets::folder_path(&path),
                    name: folder.clone(),
                }
            };
            let assets_meta = if single_file { String::new() } else { folder };
            // The instance selectors that still have a rule, space-delimited and padded so a
            // substring test cannot match `.wg-i1` inside `.wg-i10`.
            let live_handles = {
                let s = state.borrow();
                let mut list = String::from(" ");
                for key in s.custom.keys().filter(|k| docstyle::is_instance_selector(k)) {
                    list.push_str(key);
                    list.push(' ');
                }
                list
            };
            // Three fixups in the DOM before serialising: drop the editing-only outline class, give
            // the document a `<title>` matching its file name if it is still the placeholder (0.2.0
            // fixed the *window* title and left the document's own saying "Untitled" forever), and
            // record the page geometry so the file describes the shape it was written for.
            let script = format!(
                "(function (title, page, live, assets) {{
                   /* Three kinds of class must not reach the file: the two editing markers, and
                      any `wg-iN` handle no rule refers to any more. None of them may be removed
                      from the LIVE document, though -- removing the markers outright was a real
                      defect (after one Ctrl+S the style sidebar had no selected element and
                      silently stopped responding), and removing a dead handle would make the
                      style change that dropped its rule impossible to undo, because nothing could
                      find the element again to put it back. So they come off, the document is
                      serialised, and they all go straight back on. */
                   const touched = [];
                   document.querySelectorAll('[class]').forEach(function (el) {{
                     const was = el.getAttribute('class');
                     const kept = was.split(/\\s+/).filter(function (c) {{
                       if (c === 'wg-selected' || c === '{cursor}') return false;
                       if (/^wg-i\\d+$/.test(c)) return live.indexOf(' .' + c + ' ') !== -1;
                       return c.length > 0;
                     }}).join(' ');
                     if (kept === was) return;
                     touched.push([el, was]);
                     if (kept) {{ el.setAttribute('class', kept); }} else {{ el.removeAttribute('class'); }}
                   }});
                   if (title && (!document.title || document.title === 'Untitled')) {{
                     document.title = title;
                   }}
                   let m = document.querySelector('meta[name=\"{meta}\"]');
                   if (!m) {{
                     m = document.createElement('meta');
                     m.setAttribute('name', '{meta}');
                     document.head.insertBefore(m, document.head.firstChild);
                   }}
                   m.setAttribute('content', page);
                   /* The document says which folder is its own, so Word can tell you a picture is
                      missing rather than showing a silent gap. */
                   let a = document.querySelector('meta[name=\"{assets_meta_name}\"]');
                   if (assets) {{
                     if (!a) {{
                       a = document.createElement('meta');
                       a.setAttribute('name', '{assets_meta_name}');
                       document.head.insertBefore(a, document.head.firstChild);
                     }}
                     a.setAttribute('content', assets);
                   }} else if (a) {{
                     a.remove();
                   }}
                   const html = document.documentElement.outerHTML;
                   touched.forEach(function (p) {{ p[0].setAttribute('class', p[1]); }});
                   return html;
                 }})({title}, {page}, {live}, {assets})",
                meta = page::PAGE_META,
                assets_meta_name = assets::META_ASSETS,
                cursor = docstyle::CURSOR_CLASS,
                title = js::string(&title),
                page = js::string(&page_meta),
                live = js::string(&live_handles),
                assets = js::string(&assets_meta),
            );
            view.evaluate_javascript(
                &script,
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    let Ok(v) = res else {
                        say("Could not read the document back — nothing was written.");
                        return;
                    };
                    let html = v.to_str().to_string();
                    // Second sanitising pass. The first one cleaned the file; this one catches
                    // whatever arrived since — chiefly paste, which can carry a whole subtree in
                    // from a browser, script and all.
                    let (clean, report) = sanitise::clean(&html, source_dir.as_deref(), &policy);
                    // `outerHTML` does not include the doctype. Saving without it put every
                    // document Word wrote into quirks mode in every browser — measured, and the
                    // single worst bug in 0.2.0 given the whole premise is "any browser opens it".
                    let file = format!("<!doctype html>\n{clean}\n");

                    if let Err(e) = write_atomically(&path, file.as_bytes()) {
                        // The old code printed this to stderr, where nobody launching from a menu
                        // would ever see it, and carried on as though the save had happened.
                        error_dialog(
                            &window,
                            "Could not save",
                            &format!("{} could not be written: {e}", path.display()),
                        );
                        return;
                    }

                    window.set_title(Some(&format!("{} — Word", doc_title(&Some(path.clone())))));
                    // Saved == not modified. Without this the very next Close would still claim
                    // unsaved changes on a document that had just been written to disk. The
                    // baseline is the DOM as serialised, matching what `load_changed` captures --
                    // NOT the sanitised bytes, which differ by the doctype at least.
                    let mut s = state.borrow_mut();
                    s.baseline = html;
                    s.path = Some(path.clone());
                    drop(s);
                    // A converted document just got a real home — its temp preview dir is done.
                    drop_temp_convert(&state, Some(&path));

                    if let Some(summary) = report.summary() {
                        say(&summary);
                    }
                },
            );
        })
    };

    // Read the document out of the DOM, for the paths that do not go through `save_to` — the two
    // one-file exports. `save_to` does its own read because it also makes fixups on the way past.
    let read_document: Rc<dyn Fn(Rc<dyn Fn(String)>)> = {
        let view = view.clone();
        Rc::new(move |done: Rc<dyn Fn(String)>| {
            view.evaluate_javascript(
                "document.documentElement.outerHTML",
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    if let Ok(v) = res {
                        done(v.to_str().to_string());
                    }
                },
            );
        })
    };

    // Ask for a path, then save to it.
    let save_as = {
        let save_to = save_to.clone();
        let window = window.clone();
        let state = state.clone();
        let paged_active = paged_active.clone();
        let say = say.clone();
        Rc::new(move || {
            if paged_active.get() {
                say("Leave Pages view first — what is on screen is a reading layout, not the document.");
                return;
            }
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("HTML document"));
            filter.add_pattern("*.html");
            filter.add_pattern("*.htm");
            let list = gtk::gio::ListStore::new::<gtk::FileFilter>();
            list.append(&filter);
            // A docx conversion suggests its natural home — stem.html beside the original —
            // rather than the temp dir it is previewing from.
            let current = {
                let s = state.borrow();
                if s.temp_convert.is_some() { s.suggested.clone() } else { s.path.clone() }
            };
            let d = gtk::FileDialog::builder()
                .title("Save document")
                .initial_name(current.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document.html".into()))
                .filters(&list)
                .build();
            if let Some(dir) = current.as_ref().and_then(|p| p.parent()) {
                d.set_initial_folder(Some(&gtk::gio::File::for_path(dir)));
            }
            let save_to = save_to.clone();
            d.save(Some(&window), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res {
                    if let Some(p) = f.path() {
                        save_to(with_html_extension(p), false);
                    }
                }
            });
        })
    };

    let save = {
        let save_to = save_to.clone();
        let save_as = save_as.clone();
        let state = state.clone();
        let paged_active = paged_active.clone();
        let say = say.clone();
        Rc::new(move || {
            // Pages view wraps the DOM in sheet scaffolding; saving now would write the
            // scaffolding into the file. Refuse rather than silently unwrap-and-rewrap.
            if paged_active.get() {
                say("Leave Pages view first — what is on screen is a reading layout, not the document.");
                return;
            }
            // Read the path out and let the borrow go before saving: holding a RefCell borrow across
            // a call that may re-enter it is a panic waiting for someone to make the callback
            // synchronous. A docx preview's "path" is its TEMP home — plain Save must ask where
            // the document really lives, not quietly write into a dir that dies on close.
            let (path, previewing) = {
                let s = state.borrow();
                (s.path.clone(), s.temp_convert.is_some())
            };
            match path {
                Some(p) if !previewing => save_to(p, false),
                _ => save_as(),
            }
        })
    };

    {
        let save = save.clone();
        save_b.connect_clicked(move |_| save());
    }

    {
        let load_into = load_into.clone();
        let window = window.clone();
        let view = view.clone();
        let state = state.clone();
        let app = app.clone();
        let settings = settings.clone();
        open_b.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Documents (HTML, Word)"));
            filter.add_pattern("*.html");
            filter.add_pattern("*.htm");
            filter.add_pattern("*.docx");
            let list = gtk::gio::ListStore::new::<gtk::FileFilter>();
            list.append(&filter);
            let d = gtk::FileDialog::builder().title("Open document").filters(&list).build();
            let load_into = load_into.clone();
            let window2 = window.clone();
            let view = view.clone();
            let state = state.clone();
            let app = app.clone();
            let settings = settings.clone();
            d.open(Some(&window2), gtk::gio::Cancellable::NONE, move |res| {
                let Ok(f) = res else { return };
                let Some(picked) = f.path() else { return };
                // One document, one window (Piers, 2026-08-06; labwc is the tab strip). Only a
                // PRISTINE blank — never saved, never typed in — is reused; anything else keeps
                // its window and the new document gets its own. That also retires the "opening
                // replaces your work behind a confirm" gesture: nothing is replaced any more.
                let state = state.clone();
                let load_into = load_into.clone();
                let app = app.clone();
                let settings = settings.clone();
                view.evaluate_javascript(
                    "document.documentElement.outerHTML",
                    None,
                    None,
                    gtk::gio::Cancellable::NONE,
                    move |current| {
                        let current = current.map(|v| v.to_str().to_string()).unwrap_or_default();
                        let pristine = {
                            let s = state.borrow();
                            s.path.is_none()
                                && s.temp_convert.is_none()
                                && (s.baseline.is_empty() || current == s.baseline)
                        };
                        if pristine {
                            load_into(Some(picked));
                        } else {
                            build(&app, &settings, Some(picked));
                        }
                    },
                );
            });
        });
    }

    // New opens a NEW WINDOW. It used to reload the current view, which silently discarded whatever
    // was on screen -- there is no autosave, so that was a data-loss button wearing a "new" label.
    {
        let app = app.clone();
        let settings = settings.clone();
        new_b.connect_clicked(move |_| build(&app, &settings, None));
    }

    // --- picture ---------------------------------------------------------------------------------
    {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        let say = say.clone();
        image_b.connect_clicked(move |_| insert_picture(&window, &view, &state, &say));
    }

    // The Picture menu: alignment, text wrap and scale for the picture that was right-clicked. Same
    // class vocabulary as the browser's editor, so a document styled in one behaves in the other.
    {
        let view_for_menu = view.clone();
        view.connect_context_menu(move |_, menu, hit| {
            if !hit.context_is_image() {
                return false;
            }
            let submenu = webkit6::ContextMenu::new();
            let mut actions: Vec<gtk::gio::SimpleAction> = Vec::new();
            {
                let mut add = |label: &str, align: &'static str, wrap: bool, scale: Option<i64>| {
                    let name = format!("wg-img-{}", actions.len());
                    let a = gtk::gio::SimpleAction::new(&name, None);
                    let view = view_for_menu.clone();
                    a.connect_activate(move |_, _| apply_picture_layout(&view, align, wrap, scale));
                    submenu.append(&webkit6::ContextMenuItem::from_gaction(&a, label, None));
                    actions.push(a);
                };
                add("Align left", "left", false, None);
                add("Centre", "center", false, None);
                add("Align right", "right", false, None);
                submenu.append(&webkit6::ContextMenuItem::new_separator());
                add("Wrap text, left", "left", true, None);
                add("Wrap text, right", "right", true, None);
                submenu.append(&webkit6::ContextMenuItem::new_separator());
                for percent in [25, 50, 75, 100] {
                    add(&format!("Scale to {percent}%"), "", false, Some(percent));
                }
            }
            // The actions must outlive this function or the items go dead the moment they are
            // clicked. Parking them on the WebView keeps them alive while the menu can be open, and
            // the next menu replaces them.
            unsafe {
                view_for_menu.set_data("webgen-word-picture-actions", actions);
            }
            menu.append(&webkit6::ContextMenuItem::new_separator());
            menu.append(&webkit6::ContextMenuItem::with_submenu("Picture", &submenu));
            false
        });
    }

    // --- the style sidebar ------------------------------------------------------------------------
    // Click anything in the document and the sidebar shows that element's style. The click is read
    // as viewport coordinates and resolved with `elementFromPoint`, which is exact and works for
    // pictures as well as text -- the caret alone would not tell us which picture was clicked.
    let sidebar = sidebar::build(&view, &state, settings);
    {
        // Two measured facts shape this, and both look like the code is simply broken until you
        // know them:
        //
        // 1. The gesture goes on the WINDOW, not on the WebView. A GestureClick added to the
        //    WebView never fires in either phase -- WebKit's own event handling owns those events
        //    outright. The window sees them, and translating back into the WebView's coordinate
        //    space is what `elementFromPoint` wants anyway.
        // 2. It is `pressed`, not `released`. WebKit claims the event sequence in the target phase,
        //    which CANCELS this gesture before any release arrives; `pressed` fires first, while
        //    the capture phase is still ours.
        let sidebar_select = sidebar.select_at.clone();
        let view = view.clone();
        let clicks = gtk::GestureClick::new();
        clicks.set_propagation_phase(gtk::PropagationPhase::Capture);
        clicks.connect_pressed(move |_, _, x, y| {
            let Some((vx, vy)) = window_to_view(&view, x, y) else { return };
            // Widget pixels go in as they are: the scaling into CSS-viewport coordinates now
            // happens in the script, from the live viewport-to-widget ratio, which covers display
            // scaling and zoom together (see select_at_script — measured, 2026-08-07).
            sidebar_select(vx, vy);
        });
        window.add_controller(clicks);
    }
    {
        let set_open = sidebar.set_open.clone();
        style_b.connect_toggled(move |b| set_open(b.is_active()));
    }
    // Wired here rather than where the buttons are built, because undo has to be able to refresh
    // the sidebar afterwards and the sidebar does not exist yet at that point.
    {
        let (view, state, refresh) = (view.clone(), state.clone(), sidebar.refresh.clone());
        undo_b.connect_clicked(move |_| undo::undo(&view, &state, refresh.clone()));
    }
    {
        let (view, state, refresh) = (view.clone(), state.clone(), sidebar.refresh.clone());
        redo_b.connect_clicked(move |_| undo::redo(&view, &state, refresh.clone()));
    }
    {
        // Closing from inside the sidebar has to pop the header toggle back out.
        let style_b = style_b.clone();
        let is_open = sidebar.is_open.clone();
        sidebar.root.connect_child_revealed_notify(move |_| {
            style_b.set_active(is_open());
        });
    }

    // --- links ------------------------------------------------------------------------------------
    {
        let (view, window) = (view.clone(), window.clone());
        link_b.connect_clicked(move |_| link_dialog(&window, &view));
    }

    // --- tables -----------------------------------------------------------------------------------
    // One button, two jobs: inside a table it opens that table, anywhere else it starts a new one.
    // Which it is has to be asked of the document, so the whole thing is a chain of callbacks.
    {
        let view = view.clone();
        let window = window.clone();
        let settings = settings.clone();
        let say = say.clone();
        table_b.connect_clicked(move |_| {
            let (view, window, settings, say) =
                (view.clone(), window.clone(), settings.clone(), say.clone());
            view.clone().evaluate_javascript(
                &table::find_at_cursor_script(),
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    let json = res.map(|v| v.to_str().to_string()).unwrap_or_default();
                    if let Some(existing) = table::Table::from_json(&table::unescape_attr(&json)) {
                        let (view, say) = (view.clone(), say.clone());
                        table_window::open(
                            &window,
                            &settings,
                            existing.clone(),
                            false,
                            Rc::new(move |outcome| apply_table(&view, &say, outcome, Some(&existing))),
                        );
                        return;
                    }
                    // Nothing under the cursor: a new one, numbered so it cannot collide with a
                    // table whose block was deleted and re-added.
                    let (view2, window, settings, say) =
                        (view.clone(), window.clone(), settings.clone(), say.clone());
                    view.evaluate_javascript(
                        &table::highest_id_script(),
                        None,
                        None,
                        gtk::gio::Cancellable::NONE,
                        move |res| {
                            let highest = res
                                .map(|v| v.to_str().to_string())
                                .ok()
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(0);
                            let (view, say) = (view2.clone(), say.clone());
                            table_window::ask_size(
                                &window,
                                &settings,
                                highest + 1,
                                Rc::new(move |outcome| apply_table(&view, &say, outcome, None)),
                            );
                        },
                    );
                },
            );
        });
    }

    // --- close -----------------------------------------------------------------------------------
    // Close puts the FILE down and leaves the window open on a fresh blank document. Quitting to get
    // rid of a file, or opening another one just to displace it, were the only ways to do this.
    let close_document = {
        let load_into = load_into.clone();
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        Rc::new(move || {
            let load_into = load_into.clone();
            confirm_if_modified(&window, &view, &state, move || load_into(None));
        })
    };

    // The window's X is the same data-loss risk by a different gesture, so it asks the same question.
    // Returning Stop holds the window open while the dialog is up; the Close response closes it.
    {
        let view = view.clone();
        let state = state.clone();
        window.connect_close_request(move |w| {
            if state.borrow().baseline.is_empty() {
                drop_temp_convert(&state, None);
                return glib::Propagation::Proceed;
            }
            let w2 = w.clone();
            let state2 = state.clone();
            confirm_if_modified(w, &view, &state, move || {
                // Closing a docx preview — changed or not — takes its temp dir with it. What was
                // worth keeping was saved (which already cleaned up); the rest is litter.
                drop_temp_convert(&state2, None);
                w2.destroy()
            });
            glib::Propagation::Stop
        });
    }

    // --- print -----------------------------------------------------------------------------------
    // The operation has to outlive this closure: `run_dialog` returns as soon as the dialog is
    // answered, but the job itself runs on after that. Parking it here keeps it alive until WebKit
    // says it is finished.
    let printing: Rc<RefCell<Option<webkit6::PrintOperation>>> = Rc::new(RefCell::new(None));
    let print = {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        let printing = printing.clone();
        let paged_active = paged_active.clone();
        let say = say.clone();
        Rc::new(move || {
            if paged_active.get() {
                say("Leave Pages view first — printing here would print the reading layout.");
                return;
            }
            let op = webkit6::PrintOperation::new(&view);
            // THE important line. Without it the print uses GTK's defaults (US Letter, ~10mm) and
            // the document's own @page is ignored -- measured, see the module docs.
            op.set_page_setup(&state.borrow().setup.to_gtk());
            {
                let printing = printing.clone();
                op.connect_finished(move |_| {
                    printing.borrow_mut().take();
                });
            }
            {
                let printing = printing.clone();
                op.connect_failed(move |_, e| {
                    eprintln!("webgen-word: printing failed: {e}");
                    printing.borrow_mut().take();
                });
            }
            *printing.borrow_mut() = Some(op.clone());
            op.run_dialog(Some(&window));
        })
    };
    {
        let print = print.clone();
        print_b.connect_clicked(move |_| print());
    }

    // --- menu ------------------------------------------------------------------------------------
    let menu = gtk::gio::Menu::new();
    let file_section = gtk::gio::Menu::new();
    file_section.append(Some("Save As…"), Some(&action(&window, "save-as", {
        let save_as = save_as.clone();
        move || save_as()
    })));
    file_section.append(Some("Close document"), Some(&action(&window, "close-document", {
        let close_document = close_document.clone();
        move || close_document()
    })));
    menu.append_section(None, &file_section);

    let style_section = gtk::gio::Menu::new();
    style_section.append(Some("Page setup…"), Some(&action(&window, "page-setup", {
        let window = window.clone();
        let view = view.clone();
        let state = state.clone();
        let settings = settings.clone();
        move || page_setup_dialog(&window, &view, &state, &settings)
    })));
    style_section.append(Some("Style sidebar"), Some(&action(&window, "document-style", {
        let set_open = sidebar.set_open.clone();
        move || set_open(true)
    })));
    style_section.append(Some("Base style in Settings…"), Some(&action(&window, "base-style", {
        let say = say.clone();
        move || {
            // CONTRACT.md §3: the base style is rendered generically by System Settings from the
            // manifest this repo ships, so there is no second copy of that UI here to drift.
            if let Err(e) = std::process::Command::new("webgen-settings").arg("--app").arg(APP_ID).spawn() {
                say(&format!("Could not open System Settings: {e}"));
            }
        }
    })));
    menu.append_section(None, &style_section);

    // The two one-file forms. Both are "save a copy": neither changes what the document is, so
    // neither takes over the title bar or the modified state.
    // --- pages: the marker gate's controls + the whole-document PDF reader --------------------
    // --- table: structural edits where the caret is ------------------------------------------
    // In the document, not in the table window (Piers, 2026-08-06). The window stays for whole-
    // table work; these are the edits you reach for mid-sentence.
    let table_section = gtk::gio::Menu::new();
    for (label, name, op) in [
        ("Insert row above", "tbl-row-above", TableOp::RowAbove),
        ("Insert row below", "tbl-row-below", TableOp::RowBelow),
        ("Delete row", "tbl-row-del", TableOp::DeleteRow),
        ("Insert column left", "tbl-col-left", TableOp::ColumnLeft),
        ("Insert column right", "tbl-col-right", TableOp::ColumnRight),
        ("Delete column", "tbl-col-del", TableOp::DeleteColumn),
        ("Merge with cell right", "tbl-merge-right", TableOp::MergeRight),
        ("Merge with cell below", "tbl-merge-down", TableOp::MergeDown),
        ("Split merged cell", "tbl-split", TableOp::SplitCell),
    ] {
        table_section.append(
            Some(label),
            Some(&action(&window, name, {
                let view = view.clone();
                let say = say.clone();
                move || table_op(&view, &say, op)
            })),
        );
    }
    menu.append_submenu(Some("Table"), &table_section);

    let pages_section = gtk::gio::Menu::new();
    pages_section.append(Some("Allow this page to run long"), Some(&action(&window, "page-slack", {
        let view = view.clone();
        let say = say.clone();
        move || {
            view.execute_editing_command_with_argument(
                "InsertHTML",
                &format!("<hr class=\"{}\">", paginate::SLACK_CLASS),
            );
            say("This page may run to 110% before the automatic break fires.");
        }
    })));
    pages_section.append(Some("Repaginate"), Some(&action(&window, "repaginate", {
        let view = view.clone();
        let state = state.clone();
        let say = say.clone();
        move || {
            let px = paginate::inner_height_px(&state.borrow().setup);
            let say = say.clone();
            view.evaluate_javascript(
                &paginate::repaginate_script(px),
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    if let Ok(v) = res {
                        let out = v.to_str().to_string();
                        let pages = out.split(',').nth(1).unwrap_or("?").to_string();
                        say(&format!("Repaginated — {pages} page(s)."));
                    }
                },
            );
        }
    })));
    pages_section.append(Some("Read as PDF"), Some(&action(&window, "read-pdf", {
        let view = view.clone();
        let state = state.clone();
        let say = say.clone();
        let printing = printing.clone();
        let paged_active = paged_active.clone();
        move || {
            if paged_active.get() {
                say("Leave Pages view first — the PDF prints the document, not the reading layout.");
                return;
            }
            // The whole document, paginated by its markers, in webgen-pdf — reading mode without
            // teaching the editor to page. The file is a throwaway in the cache; the reader holds
            // it open and the next Read overwrites it.
            let out = gtk::glib::user_cache_dir().join("webgen-word");
            let _ = std::fs::create_dir_all(&out);
            let pdf = out.join(format!("read-{}.pdf", std::process::id()));
            let op = webkit6::PrintOperation::new(&view);
            op.set_page_setup(&state.borrow().setup.to_gtk());
            let ps = gtk::PrintSettings::new();
            ps.set(gtk::PRINT_SETTINGS_PRINTER, Some("Print to File"));
            ps.set(gtk::PRINT_SETTINGS_OUTPUT_URI, Some(&format!("file://{}", pdf.display())));
            ps.set(gtk::PRINT_SETTINGS_OUTPUT_FILE_FORMAT, Some("pdf"));
            op.set_print_settings(&ps);
            {
                let say = say.clone();
                let printing = printing.clone();
                let pdf = pdf.clone();
                op.connect_finished(move |_| {
                    printing.borrow_mut().take();
                    match std::process::Command::new("webgen-pdf").arg(&pdf).spawn() {
                        Ok(_) => say("Opened in the PDF reader."),
                        Err(e) => say(&format!("PDF written to {} (reader: {e})", pdf.display())),
                    }
                });
            }
            {
                let say = say.clone();
                op.connect_failed(move |_, e| say(&format!("Could not write the PDF: {e}")));
            }
            *printing.borrow_mut() = Some(op.clone());
            op.print();
        }
    })));
    menu.append_section(None, &pages_section);

    let export_section = gtk::gio::Menu::new();
    export_section.append(Some("Save a copy as one file…"), Some(&action(&window, "export-single", {
        let (window, save_to) = (window.clone(), save_to.clone());
        let state = state.clone();
        move || {
            let initial = state
                .borrow()
                .path
                .as_ref()
                .and_then(|p| p.file_stem().map(|s| format!("{}-single.html", s.to_string_lossy())))
                .unwrap_or_else(|| "document-single.html".into());
            let save_to = save_to.clone();
            ask_for_path(&window, "Save a copy as one file", "HTML document", &["*.html", "*.htm"], &initial, move |p| {
                save_to(with_extension(p, "html"), true);
            });
        }
    })));
    export_section.append(Some("Save a copy as Word (.docx)…"), Some(&action(&window, "export-docx", {
        let (window, state, say) = (window.clone(), state.clone(), say.clone());
        let read_document = read_document.clone();
        move || {
            let initial = state
                .borrow()
                .path
                .as_ref()
                .and_then(|p| p.file_stem().map(|s| format!("{}.docx", s.to_string_lossy())))
                .unwrap_or_else(|| "document.docx".into());
            let (state, say, read_document) = (state.clone(), say.clone(), read_document.clone());
            ask_for_path(&window, "Save a copy as Word", "Word document", &["*.docx"], &initial, move |p| {
                let target = with_extension(p, "docx");
                let setup = state.borrow().setup;
                // Pictures come from the document's own folder, resolved the same way the sanitiser
                // resolves them: beside the file it was loaded from.
                let source_dir = state.borrow().path.as_ref().and_then(|d| d.parent().map(|x| x.to_path_buf()));
                let say = say.clone();
                read_document(Rc::new(move |html: String| {
                    let parsed = webgen_convert::from_html::parse(&html);
                    let mut media = std::collections::HashMap::new();
                    if let Some(dir) = &source_dir {
                        // The pictures folder is `<stem>_files`; a name that escapes it is refused
                        // rather than followed.
                        for name in &parsed.images {
                            if name.contains('/') || name.contains('\\') || name.starts_with('.') {
                                continue;
                            }
                            let mut found = None;
                            if let Ok(entries) = std::fs::read_dir(dir) {
                                for e in entries.flatten() {
                                    let candidate = e.path().join(name);
                                    if candidate.is_file() {
                                        found = std::fs::read(&candidate).ok();
                                        break;
                                    }
                                }
                            }
                            if let Some(bytes) = found {
                                media.insert(name.clone(), bytes);
                            }
                        }
                    }
                    let page = webgen_convert::to_docx::PageOut {
                        width_mm: setup.paper.width_mm(),
                        height_mm: setup.paper.height_mm(),
                        top_mm: setup.top,
                        right_mm: setup.right,
                        bottom_mm: setup.bottom,
                        left_mm: setup.left,
                    };
                    match webgen_convert::to_docx::write_docx(&parsed.nodes, &media, page)
                        .and_then(|bytes| std::fs::write(&target, bytes).map_err(|e| e.to_string()))
                    {
                        Ok(()) => say(&format!(
                            "Saved {} — {} block(s), {} picture(s).",
                            target.display(),
                            parsed.nodes.len(),
                            media.len()
                        )),
                        Err(e) => say(&format!("Could not write {}: {e}", target.display())),
                    }
                }));
            });
        }
    })));
    export_section.append(Some("Save a copy as .wgz…"), Some(&action(&window, "export-wgz", {
        let (window, state, say) = (window.clone(), state.clone(), say.clone());
        let read_document = read_document.clone();
        move || {
            let initial = state
                .borrow()
                .path
                .as_ref()
                .and_then(|p| p.file_stem().map(|s| format!("{}.wgz", s.to_string_lossy())))
                .unwrap_or_else(|| format!("document.{}", assets::ZIP_EXTENSION));
            let (window2, state, say, read_document) =
                (window.clone(), state.clone(), say.clone(), read_document.clone());
            ask_for_path(&window, "Save a copy as .wgz", "WebGen document", &["*.wgz"], &initial, move |p| {
                let target = with_extension(p, assets::ZIP_EXTENSION);
                let source_dir = state.borrow().path.as_ref().and_then(|d| d.parent().map(|x| x.to_path_buf()));
                let say = say.clone();
                let _ = &window2;
                read_document(Rc::new(move |html: String| {
                    match write_wgz(&target, &html, source_dir.as_deref()) {
                        Ok(count) => say(&format!(
                            "Saved {} — {count} file(s) inside, ready to send.",
                            target.display()
                        )),
                        Err(e) => say(&format!("Could not write {}: {e}", target.display())),
                    }
                }));
            });
        }
    })));
    menu.append_section(None, &export_section);
    menu_b.set_menu_model(Some(&menu));

    // --- keyboard ---------------------------------------------------------------------------------
    // CAPTURE phase, so these beat the WebView. Only bindings WebKit does NOT already provide are
    // added: it handles Ctrl+B/I/U, Ctrl+Z/Y and the clipboard itself, and binding those again here
    // would fire the command twice and un-toggle it.
    // View-only zoom (Piers, 2026-08-06). WebKit's own zoom level: the document scales, the file
    // and the print/PDF path are untouched. Clamped so a stray key cannot zoom the page away.
    let zoom = {
        let view = view.clone();
        let say = say.clone();
        Rc::new(move |step: f64| {
            let z = if step == 0.0 {
                1.0
            } else {
                (view.zoom_level() + step).clamp(0.5, 3.0)
            };
            view.set_zoom_level(z);
            say(&format!("Zoom {:.0}%", z * 100.0));
        })
    };

    {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let view_k = view.clone();
        let zoom = zoom.clone();
        let pages_toggle = pages_b.clone();
        let new_click = new_b.clone();
        let open_click = open_b.clone();
        let print = print.clone();
        let save = save.clone();
        let save_as = save_as.clone();
        let close_document = close_document.clone();
        let brk_click = brk.clone();
        let link_click = link_b.clone();
        let undo_click = undo_b.clone();
        let undo_redo_b = redo_b.clone();
        keys.connect_key_pressed(move |_, key, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            use gtk::gdk::Key;
            match (ctrl, shift, key) {
                // Taken off WebKit deliberately. It handles Ctrl+Z and Ctrl+Shift+Z itself, and
                // its stack knows nothing about style changes -- so these have to be intercepted
                // here or the buttons and the keys would do different things.
                (true, true, Key::Z | Key::z) => undo_redo_b.emit_clicked(),
                (true, false, Key::z) => undo_click.emit_clicked(),
                (true, false, Key::y) => undo_redo_b.emit_clicked(),
                (true, true, Key::S | Key::s) => save_as(),
                (true, false, Key::n) => new_click.emit_clicked(),
                (true, false, Key::o) => open_click.emit_clicked(),
                (true, false, Key::s) => save(),
                (true, false, Key::p) => print(),
                (true, false, Key::w) => close_document(),
                (true, false, Key::k) => link_click.emit_clicked(),
                // Ctrl+= is what the unshifted plus key actually sends; both are accepted.
                (true, _, Key::plus | Key::equal | Key::KP_Add) => zoom(0.1),
                (true, false, Key::minus | Key::KP_Subtract) => zoom(-0.1),
                (true, false, Key::_0 | Key::KP_0) => zoom(0.0),
                // Esc leaves Pages view; anywhere else it stays WebKit's.
                (false, false, Key::Escape) if pages_toggle.is_active() => {
                    pages_toggle.set_active(false)
                }
                (true, _, Key::Return | Key::KP_Enter) => brk_click.emit_clicked(),
                // Tab indents, Shift+Tab outdents. In a list this is how a sub-list is made, which
                // is the behaviour every word processor has and the reason Tab does not insert a
                // tab character here.
                //
                // **Only while the document has focus.** This controller is on the window and in the
                // capture phase, so before 0.3.0 it swallowed Tab everywhere: the toolbar could not
                // be reached from the keyboard at all, and Tab indented the document while focus was
                // on a button.
                (false, false, Key::Tab) if view_k.has_focus() => view_k.execute_editing_command("Indent"),
                (false, _, Key::ISO_Left_Tab) if view_k.has_focus() => {
                    view_k.execute_editing_command("Outdent")
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        window.add_controller(keys);
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&banner);
    content.append(&fmt);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let scroller = gtk::ScrolledWindow::builder().child(&view).vexpand(true).hexpand(true).build();
    // The document and the style sidebar sit side by side; the sidebar slides in when it is opened.
    let middle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    middle.append(&scroller);
    middle.append(&sidebar.root);
    content.append(&middle);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&content));
    window.set_content(Some(&tv));
    // The 94% gate (see `paginate`). A light timer, not a keystroke hook: measuring the page on
    // every key would fight the typing, and a marker landing up to a couple of seconds after the
    // line that crossed the window is exactly as useful. Runs only while this window is the
    // active one; ends with the window.
    {
        let view = view.clone();
        let state = state.clone();
        let say = say.clone();
        let win_weak = window.downgrade();
        let paged_active = paged_active.clone();
        let last_pages: Rc<std::cell::Cell<i64>> = Rc::new(std::cell::Cell::new(1));
        glib::timeout_add_local(std::time::Duration::from_millis(2500), move || {
            let Some(win) = win_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !win.is_active() || state.borrow().baseline.is_empty() || paged_active.get() {
                // Paused in Pages view: the sheets change every offset the gate measures by.
                return glib::ControlFlow::Continue;
            }
            let px = paginate::inner_height_px(&state.borrow().setup);
            let say = say.clone();
            let last_pages = last_pages.clone();
            view.evaluate_javascript(
                &paginate::check_script(px),
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    let Ok(v) = res else { return };
                    let out = v.to_str().to_string();
                    let mut parts = out.split(',');
                    let inserted: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let pages: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                    if inserted > 0 || pages != last_pages.get() {
                        last_pages.set(pages);
                        say(&format!("{pages} page(s)."));
                    }
                },
            );
            glib::ControlFlow::Continue
        });
    }

    window.present();
}

/// Put what the Table Window decided into the document.
///
/// Saving an existing table replaces everything between its boundary markers; saving a new one puts
/// a block in at the caret. Deleting takes the block out. In every case the markup is *generated*
/// from the model rather than patched, which is what keeps the document and the JSON in step.
/// The structural table edits offered in the document itself.
#[derive(Clone, Copy, PartialEq)]
enum TableOp {
    RowAbove,
    RowBelow,
    DeleteRow,
    ColumnLeft,
    ColumnRight,
    DeleteColumn,
    MergeRight,
    MergeDown,
    SplitCell,
}

/// Apply a structural edit to the table the CARET is in (Piers, 2026-08-06: in the document, not
/// in the table window). The model is the truth: the block is regenerated from it, so the JSON,
/// the markup and the scoped CSS cannot drift apart. Nothing happens — with a reason in the
/// status line — when the caret is not in a table.
fn table_op(view: &webkit6::WebView, say: &Rc<impl Fn(&str) + 'static>, op: TableOp) {
    let view2 = view.clone();
    let say = say.clone();
    view.evaluate_javascript(
        &table::find_cell_at_cursor_script(),
        None,
        None,
        gtk::gio::Cancellable::NONE,
        move |res| {
            let found = res.map(|v| v.to_str().to_string()).unwrap_or_default();
            let mut parts = found.splitn(4, '|');
            let (Some(section), Some(row), Some(index), Some(json)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                say("Put the cursor in a table first.");
                return;
            };
            let (row, index) = (
                row.parse::<usize>().unwrap_or(0),
                index.parse::<usize>().unwrap_or(0),
            );
            let Some(mut table) = table::Table::from_json(&table::unescape_attr(json)) else {
                say("That table's data could not be read — nothing was changed.");
                return;
            };

            // Row operations act on the body: a header row is the table's shape, not a row you
            // add another of by accident. A caret in the header means "the top of the body".
            let in_body = section == "body";
            let body_row = if in_body { row } else { 0 };
            // The caret's GRID column, which is what column operations and merges are indexed
            // by: a row carrying spans has fewer cells than the table has columns.
            let grid = table::Grid::of(match section {
                "head" => &table.head,
                "foot" => &table.foot,
                _ => &table.body,
            });
            let placed = grid.cells.iter().find(|c| c.row == row && c.index == index);
            let col = placed.map(|c| c.col).unwrap_or(0);
            let span = placed.map(|c| c.colspan).unwrap_or(1);

            let done = match op {
                TableOp::RowAbove => {
                    table.insert_row(body_row);
                    "Row added."
                }
                TableOp::RowBelow => {
                    table.insert_row(if in_body { body_row + 1 } else { 0 });
                    "Row added."
                }
                TableOp::DeleteRow => {
                    if !in_body {
                        say("A header row cannot be deleted from here — use the table window.");
                        return;
                    }
                    table.delete_row(body_row);
                    "Row deleted."
                }
                TableOp::ColumnLeft => {
                    table.insert_column(col);
                    "Column added."
                }
                TableOp::ColumnRight => {
                    table.insert_column(col + span);
                    "Column added."
                }
                TableOp::DeleteColumn => {
                    table.delete_column(col);
                    "Column deleted."
                }
                TableOp::MergeRight => {
                    if !in_body || !table.merge(body_row, col, body_row, col + span) {
                        say("Those cells cannot be merged.");
                        return;
                    }
                    "Cells merged."
                }
                TableOp::MergeDown => {
                    if !in_body || !table.merge(body_row, col, body_row + 1, col) {
                        say("Those cells cannot be merged.");
                        return;
                    }
                    "Cells merged."
                }
                TableOp::SplitCell => {
                    if !in_body || !table.split(body_row, col) {
                        say("That cell is not merged.");
                        return;
                    }
                    "Cell split."
                }
            };

            let script = table::replace_block_script(&table.class(), &table.to_block());
            let say2 = say.clone();
            view2.evaluate_javascript(&script, None, None, gtk::gio::Cancellable::NONE, move |r| {
                let ok = r.map(|v| v.to_str().to_string()).unwrap_or_default();
                if ok.is_empty() {
                    say2("The table could not be updated — nothing was changed.");
                } else {
                    say2(done);
                }
            });
        },
    );
}

fn apply_table(
    view: &webkit6::WebView,
    say: &Rc<impl Fn(&str) + 'static>,
    outcome: table_window::Outcome,
    existing: Option<&table::Table>,
) {
    let script = match (&outcome, existing) {
        (table_window::Outcome::Save(table), Some(old)) => {
            table::replace_block_script(&old.class(), &table.to_block())
        }
        (table_window::Outcome::Save(table), None) => table::insert_block_script(&table.to_block()),
        (table_window::Outcome::Delete, Some(old)) => table::replace_block_script(&old.class(), ""),
        // Deleting a table that was never inserted is just not inserting it.
        (table_window::Outcome::Delete, None) => return,
    };
    let say = say.clone();
    view.evaluate_javascript(&script, None, None, gtk::gio::Cancellable::NONE, move |res| {
        let ok = res.map(|v| v.to_str().to_string()).unwrap_or_default();
        if ok.is_empty() {
            say("The table could not be placed in the document — nothing was changed.");
        }
    });
}

/// Translate a click on the window into the WebView's own coordinates, or `None` when it landed
/// somewhere else — the toolbar, the sidebar, the header.
fn window_to_view(view: &webkit6::WebView, x: f64, y: f64) -> Option<(f64, f64)> {
    let root = view.root()?;
    let (vx, vy) = root.translate_coordinates(view, x, y)?;
    let inside = vx >= 0.0 && vy >= 0.0 && vx < view.width() as f64 && vy < view.height() as f64;
    inside.then_some((vx, vy))
}

/// Make, change or remove a link on the selection.
///
/// Links are the one outward reference a document is allowed to keep — a link is a reference, not
/// an imported asset, which is why the sanitiser has always let them through. Until now there was
/// simply no way to make one.
fn link_dialog(window: &adw::ApplicationWindow, view: &webkit6::WebView) {
    let window = window.clone();
    let view = view.clone();
    // Ask the document what is selected first, so the dialog opens on the truth: the existing href
    // if the caret is already in a link, and the selected words if it is not.
    view.clone().evaluate_javascript(
        "(function () {
           const sel = window.getSelection();
           let node = sel && sel.anchorNode;
           let el = node ? (node.nodeType === 1 ? node : node.parentElement) : null;
           while (el && el.tagName !== 'A') el = el.parentElement;
           const href = el ? (el.getAttribute('href') || '') : '';
           const text = sel ? sel.toString() : '';
           return href + String.fromCharCode(31) + text;
         })()",
        None,
        None,
        gtk::gio::Cancellable::NONE,
        move |res| {
            let record = res.map(|v| v.to_str().to_string()).unwrap_or_default();
            let mut parts = record.splitn(2, '\u{1f}');
            let existing = parts.next().unwrap_or("").to_string();
            let selected = parts.next().unwrap_or("").to_string();
            show_link_dialog(&window, &view, existing, selected);
        },
    );
}

fn show_link_dialog(
    window: &adw::ApplicationWindow,
    view: &webkit6::WebView,
    existing: String,
    selected: String,
) {
    let dialog = adw::Window::builder()
        .transient_for(window)
        .modal(true)
        .title(if existing.is_empty() { "Add link" } else { "Edit link" })
        .default_width(460)
        .build();

    let group = adw::PreferencesGroup::new();
    group.set_description(Some(
        "A web address, or # and a name to jump somewhere in this document.",
    ));
    let url = adw::EntryRow::new();
    url.set_title("Links to");
    url.set_text(&existing);
    group.add(&url);

    let text = adw::EntryRow::new();
    text.set_title("Text to show");
    text.set_text(&selected);
    // Only offered when there is nothing selected to turn into a link.
    if !selected.is_empty() {
        text.set_sensitive(false);
        text.set_tooltip_text(Some("The selected words are what the link will show"));
    }
    group.add(&text);

    let cancel = gtk::Button::with_label("Cancel");
    let remove = gtk::Button::with_label("Remove link");
    remove.add_css_class("destructive-action");
    remove.set_visible(!existing.is_empty());
    let apply = gtk::Button::with_label(if existing.is_empty() { "Add link" } else { "Update" });
    apply.add_css_class("suggested-action");

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let (dialog, view) = (dialog.clone(), view.clone());
        remove.connect_clicked(move |_| {
            view.execute_editing_command("Unlink");
            dialog.close();
        });
    }
    {
        let (dialog, view, url, text) = (dialog.clone(), view.clone(), url.clone(), text.clone());
        let had_selection = !selected.is_empty();
        apply.connect_clicked(move |_| {
            let href = url.text().trim().to_string();
            if href.is_empty() {
                return;
            }
            if had_selection {
                view.execute_editing_command_with_argument("CreateLink", &href);
            } else {
                // Nothing selected: put the words in first, then link them. Without this, linking
                // an empty selection produces a link with no text — invisible, and unclickable.
                let label = {
                    let typed = text.text().trim().to_string();
                    if typed.is_empty() { href.clone() } else { typed }
                };
                let script = format!(
                    "document.execCommand('insertHTML', false, {})",
                    js::string(&format!(
                        "<a href=\"{}\">{}</a>",
                        doc::escape_attr(&href),
                        doc::escape(&label)
                    ))
                );
                view.evaluate_javascript(&script, None, None, gtk::gio::Cancellable::NONE, |_| {});
            }
            dialog.close();
        });
    }

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&remove);
    buttons.append(&apply);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.append(&group);
    content.append(&buttons);

    let view_bar = adw::ToolbarView::new();
    view_bar.add_top_bar(&adw::HeaderBar::new());
    view_bar.set_content(Some(&content));
    dialog.set_content(Some(&view_bar));
    dialog.present();
}

/// Is this one of ours?
fn is_wgz(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(assets::ZIP_EXTENSION))
        .unwrap_or(false)
}

/// Unpack a `.wgz` into a folder beside it, and return the document to open and where it went.
///
/// Entry names are checked rather than trusted: a zip can name a file `../../.bashrc`, and
/// unpacking is exactly where that matters.
fn unpack_wgz(archive: &Path) -> Result<(PathBuf, PathBuf), String> {
    let bytes = std::fs::read(archive).map_err(|e| e.to_string())?;
    let entries = assets::read_zip(&bytes).map_err(|e| e.to_string())?;
    let stem = archive
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".into());
    let target = archive.with_file_name(&stem);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;

    let mut document: Option<PathBuf> = None;
    for (name, data) in entries {
        let mut safe = target.clone();
        for part in name.split('/') {
            if part.is_empty() || part == "." || part == ".." {
                return Err(format!("{name} is not a name this can unpack safely"));
            }
            safe.push(assets::sanitise_name(part));
        }
        if let Some(parent) = safe.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&safe, data).map_err(|e| e.to_string())?;
        if document.is_none()
            && safe.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("html")).unwrap_or(false)
        {
            document = Some(safe);
        }
    }
    document.map(|d| (d, target)).ok_or_else(|| "there is no document inside it".to_string())
}

fn is_docx(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("docx"))
        .unwrap_or(false)
}

/// How a `.docx` conversion treats the source document's visual styling.
#[derive(Clone, Copy, PartialEq)]
enum ConvertMode {
    /// Honour the source: its borders, its cell shading — mapped into scoped sheet rules.
    Document,
    /// Ignore the source's look; give every table Word's own chrome.
    System,
    /// Structure only: no borders, no shading. What the sanitiser would leave of a bare paste.
    Plain,
}

impl ConvertMode {
    fn key(self) -> &'static str {
        match self {
            ConvertMode::Document => "document",
            ConvertMode::System => "system",
            ConvertMode::Plain => "plain",
        }
    }
    fn from_key(s: &str) -> ConvertMode {
        match s {
            "system" => ConvertMode::System,
            "plain" => ConvertMode::Plain,
            _ => ConvertMode::Document,
        }
    }
}

/// Map a converter table into a native table block when every cell is simple enough to adopt
/// losslessly. A native block is the prize: the table window and the CSS panel both work on it.
fn native_table(t: &webgen_convert::DocTable, id: u32, mode: ConvertMode) -> Option<table::Table> {
    let mut rows: Vec<Vec<table::Cell>> = Vec::new();
    for row in &t.rows {
        let mut cells = Vec::new();
        for c in row {
            let (text, bold) = c.simple.clone()?;
            cells.push(table::Cell {
                text,
                bold,
                colspan: c.colspan as usize,
                rowspan: c.rowspan as usize,
                fill: match mode {
                    ConvertMode::Document => c.fill.clone().unwrap_or_default(),
                    _ => String::new(),
                },
                ..Default::default()
            });
        }
        rows.push(cells);
    }
    if rows.is_empty() {
        return None;
    }
    // The document's own column widths, adopted whole. Plain mode drops them: "no formatting"
    // means the page decides, not the source.
    let cols = if mode == ConvertMode::Plain { Vec::new() } else { t.col_widths_mm.clone() };
    let head = vec![rows.remove(0)];
    let mut css = std::collections::BTreeMap::new();
    let bordered = match mode {
        ConvertMode::Document => t.bordered,
        ConvertMode::System => true,
        ConvertMode::Plain => false,
    };
    if bordered {
        // 1px black across both bordered modes, matching what render_table_html gives the
        // complex tables — one document, one look. (User-created tables keep Table::new's
        // softer grey; a converted form is imitating paper, not the app.)
        let cell = crate::docstyle::TagStyle {
            border: "1px solid #000000".into(),
            padding: "6px".into(),
            ..Default::default()
        };
        css.insert("th".to_string(), crate::docstyle::TagStyle { font_weight: "bold".into(), ..cell.clone() });
        css.insert("td".to_string(), cell);
    }
    Some(table::Table { id, head, body: rows, foot: Vec::new(), cols, css })
}

/// Delete a docx conversion's temporary home, unless `keep_if_under` still points into it.
/// Called whenever the document it held stops being what is on screen: closed, replaced, or
/// saved to a real home. Looking at a document must not litter (Piers, 2026-08-06).
fn drop_temp_convert(state: &Rc<RefCell<State>>, keep_if_under: Option<&Path>) {
    let dir = state.borrow().temp_convert.clone();
    let Some(dir) = dir else { return };
    if let Some(k) = keep_if_under {
        if k.starts_with(&dir) {
            return;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = state.borrow_mut();
    s.temp_convert = None;
    s.suggested = None;
}

/// Remove convert dirs a crashed session left behind. Anything older than two days cannot be on
/// anyone's screen; a live one belongs to another running window and is younger than that.
fn sweep_stale_converts() {
    let base = gtk::glib::user_cache_dir().join("webgen-word");
    let Ok(entries) = std::fs::read_dir(&base) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        if !name.to_string_lossy().starts_with("convert-") {
            continue;
        }
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age.as_secs() > 2 * 24 * 3600)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Convert a `.docx` into html-plus-folder in a fresh TEMP dir and hand back the html to open,
/// its temp home, and the natural save target (`stem.html` beside the original).
///
/// Temp, not beside the original: conversion is a PREVIEW until the user saves. [Save] copies the
/// document (pictures included — `save_to` relocates them) to wherever the user says, and the
/// temp home is deleted; closing without saving deletes every trace. The ordinary open path
/// (sanitiser included) treats the temp html like any HTML it does not trust yet.
/// Tables whose cells are simple become NATIVE table blocks (editable in the table window, styled
/// through the CSS panel); the rest keep semantic markup with the same scoped-sheet conventions.
fn convert_docx(
    archive: &Path,
    mode: ConvertMode,
) -> Result<(PathBuf, PathBuf, PathBuf, usize, Option<PageSetup>), String> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);

    let bytes = std::fs::read(archive).map_err(|e| e.to_string())?;
    let stem = archive
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".into());
    let tmp_dir = gtk::glib::user_cache_dir().join("webgen-word").join(format!(
        "convert-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    let target = tmp_dir.join(format!("{stem}.html"));
    let suggested = archive.with_file_name(format!("{stem}.html"));
    let folder = assets::folder_name(&target);
    let out = webgen_convert::docx_to_segments(&bytes, &folder).map_err(|e| e.to_string())?;

    let mut body = String::new();
    // The document's header and footer, once at the top and once at the bottom. HTML has no page
    // model — WebKit's print path supports no @page margin boxes — so per-page repetition is not
    // expressible; showing them once is honest and puts the banner and logo back (Piers,
    // 2026-08-06: "do 1 now"). Marked with their own classes so a stylesheet can hide or restyle
    // them, and so a later docx export can put them back where they came from.
    if mode != ConvertMode::Plain && !out.header_html.trim().is_empty() {
        body.push_str(&format!(
            "<div class=\"wg-doc-header\">{}</div>\n",
            out.header_html
        ));
    }
    let mut native_id = 0u32;
    for seg in out.segments {
        match seg {
            webgen_convert::Segment::Html(h) => body.push_str(&h),
            webgen_convert::Segment::Table(mut t) => {
                if let Some(native) = native_table(&t, native_id + 1, mode) {
                    native_id += 1;
                    body.push_str(&native.to_block());
                } else {
                    match mode {
                        ConvertMode::Document => {}
                        ConvertMode::System => t.bordered = true,
                        ConvertMode::Plain => {
                            t.bordered = false;
                            for row in &mut t.rows {
                                for c in row {
                                    c.fill = None;
                                }
                            }
                        }
                    }
                    body.push_str(&webgen_convert::render_table_html(&t));
                }
            }
        }
    }

    if mode != ConvertMode::Plain && !out.footer_html.trim().is_empty() {
        body.push_str(&format!(
            "<div class=\"wg-doc-footer\">{}</div>\n",
            out.footer_html
        ));
    }

    if !out.assets.is_empty() {
        let adir = assets::folder_path(&target);
        std::fs::create_dir_all(&adir).map_err(|e| e.to_string())?;
        for (name, data) in &out.assets {
            // The converter promises plain basenames; hold it to that rather than trust it.
            if name.contains('/') || name.contains('\\') || name.starts_with('.') {
                return Err(format!("{name} is not a name this can write safely"));
            }
            std::fs::write(adir.join(name), data).map_err(|e| e.to_string())?;
        }
    }
    let html = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>{}</title></head>\n<body>\n{}\n</body></html>\n",
        doc::escape(&stem),
        body
    );
    std::fs::write(&target, html).map_err(|e| e.to_string())?;
    // The document's OWN page geometry, when it carries one and the paper is one we offer.
    // Printing a converted form at the app's A4/20mm default instead of the template's real
    // margins cost ~25% of the content area — the "prints to more pages than Word does" report
    // (2026-08-06). Margins are clamped to what the panel itself allows.
    let setup = out.page.and_then(|p| {
        page::Paper::nearest(p.width_mm, p.height_mm).map(|paper| {
            let cap = |v: f64| v.clamp(0.0, 60.0).round();
            PageSetup {
                paper,
                top: cap(p.top_mm),
                right: cap(p.right_mm),
                bottom: cap(p.bottom_mm),
                left: cap(p.left_mm),
            }
        })
    });
    Ok((target, tmp_dir, suggested, out.assets.len(), setup))
}

/// Ask for a path with one filter, then hand it back.
fn ask_for_path(
    window: &adw::ApplicationWindow,
    title: &str,
    filter_name: &str,
    patterns: &[&str],
    initial: &str,
    done: impl Fn(PathBuf) + 'static,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(filter_name));
    for pattern in patterns {
        filter.add_pattern(pattern);
    }
    let list = gtk::gio::ListStore::new::<gtk::FileFilter>();
    list.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .initial_name(initial)
        .filters(&list)
        .build();
    dialog.save(Some(window), gtk::gio::Cancellable::NONE, move |res| {
        if let Ok(file) = res {
            if let Some(path) = file.path() {
                done(path);
            }
        }
    });
}

/// Give a path the extension it needs when the user typed a bare name.
fn with_extension(path: PathBuf, wanted: &str) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) {
        Some(e) if e == wanted => path,
        _ => {
            let mut name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "document".into());
            name.push('.');
            name.push_str(wanted);
            path.with_file_name(name)
        }
    }
}

/// Pack a document and its assets folder into a `.wgz`.
///
/// The document is written out normally into a scratch folder first, so the packing uses exactly
/// the same code the ordinary save does and cannot drift from it.
fn write_wgz(target: &Path, html_source: &str, source_dir: Option<&Path>) -> std::io::Result<usize> {
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".into());
    let scratch = std::env::temp_dir().join(format!("wgword-pack-{}-{stem}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;

    let folder = format!("{stem}_files");
    let policy = sanitise::AssetPolicy::Folder {
        dir: scratch.join(&folder),
        name: folder.clone(),
    };
    let (clean, _) = sanitise::clean(html_source, source_dir, &policy);

    let count = assets::pack(
        target,
        &format!("{stem}.html"),
        &format!("<!doctype html>\n{clean}\n"),
        &scratch.join(&folder),
        &folder,
    )?;
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(count)
}

/// Give a path an `.html` extension when the user typed a bare name. Saving `cv` and getting a file
/// nothing associates with anything is not what was meant.
fn with_html_extension(path: PathBuf) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) {
        Some(e) if e == "html" || e == "htm" => path,
        _ => {
            let mut name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document".into());
            name.push_str(".html");
            path.with_file_name(name)
        }
    }
}

/// Write via a temporary file and a rename.
///
/// A word processor writing straight over the document is one power cut away from having neither
/// the old version nor the new one. `rename` within a directory is atomic, so the file on disk is
/// either entirely the old document or entirely the new one.
fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document".into());
    let tmp = dir.join(format!(".{name}.wgword-tmp"));
    std::fs::write(&tmp, data)?;
    // Keep whatever the original was: a document that was read-only for the group should not become
    // group-writable because it was edited.
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn error_dialog(window: &adw::ApplicationWindow, heading: &str, body: &str) {
    let dlg = adw::MessageDialog::new(Some(window), Some(heading), Some(body));
    dlg.add_response("ok", "OK");
    dlg.set_default_response(Some("ok"));
    dlg.set_close_response("ok");
    dlg.connect_response(None, |dlg, _| dlg.close());
    dlg.present();
}

/// Pick a picture and put it in the document.
///
/// Where it goes depends on whether the document has anywhere to put it yet:
///
/// - **Saved document** — the file is copied into `<stem>_files/` under its own name, and the
///   markup points at it. That is what makes the template workflow work: the picture is a real file
///   called `front.png` that can be saved over from Paint without opening Word at all.
/// - **Never saved** — there is no folder yet, so it comes in as a `data:` URI carrying its
///   intended name in `data-wg-name`. The first save writes it out under that name.
fn insert_picture(
    window: &adw::ApplicationWindow,
    view: &webkit6::WebView,
    state: &Rc<RefCell<State>>,
    say: &Rc<impl Fn(&str) + 'static>,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Pictures"));
    for suffix in ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"] {
        filter.add_suffix(suffix);
    }
    let list = gtk::gio::ListStore::new::<gtk::FileFilter>();
    list.append(&filter);
    let d = gtk::FileDialog::builder()
        .title("Insert picture")
        .filters(&list)
        .default_filter(&filter)
        .modal(true)
        .build();
    let view = view.clone();
    let say = say.clone();
    let state = state.clone();
    d.open(Some(window), gtk::gio::Cancellable::NONE, move |res| {
        let Ok(file) = res else { return };
        let Some(source) = file.path() else { return };
        let name = source.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let doc = state.borrow().path.clone();

        let (markup, policy) = match doc.as_ref() {
            Some(doc_path) => (
                format!("<img src=\"{}\" class=\"wg-center\">", doc::escape_attr(&source.display().to_string())),
                sanitise::AssetPolicy::Folder {
                    dir: assets::folder_path(doc_path),
                    name: assets::folder_name(doc_path),
                },
            ),
            None => (
                format!(
                    "<img src=\"{}\" {}=\"{}\" class=\"wg-center\">",
                    doc::escape_attr(&source.display().to_string()),
                    sanitise::NAME_ATTR,
                    doc::escape_attr(&assets::sanitise_name(&name))
                ),
                sanitise::AssetPolicy::Embed,
            ),
        };

        // The same code that places pictures on save places this one, so an inserted picture and
        // one found in an opened document are treated identically.
        let (clean, report) = sanitise::clean(&markup, source.parent(), &policy);
        if report.embedded == 0 {
            say(&format!("{} could not be read, or is too large to embed.", source.display()));
            return;
        }
        view.execute_editing_command_with_argument("InsertHTML", &clean);
    });
}

/// Alignment, text wrap and scale for the picture the Picture menu was opened on.
fn apply_picture_layout(view: &webkit6::WebView, align: &str, wrap: bool, scale: Option<i64>) {
    let script = format!(
        "(function (align, wrap, scale) {{
           const img = document.querySelector('.wg-selected');
           if (!img) return;
           if (align) {{
             img.classList.remove('wg-left', 'wg-right', 'wg-center', 'wg-wrap');
             img.classList.add('wg-' + align);
             if (wrap) img.classList.add('wg-wrap');
           }}
           if (scale) {{
             img.style.width = scale + '%';
             img.style.height = 'auto';
           }}
         }})({align}, {wrap}, {scale})",
        align = js::string(align),
        wrap = if wrap { "true" } else { "false" },
        scale = scale.map(|s| s.to_string()).unwrap_or_else(|| "0".into()),
    );
    view.evaluate_javascript(&script, None, None, gtk::gio::Cancellable::NONE, |_| {});
}

/// Ask before throwing work away — and only when there is work to throw away.
///
/// "Modified" is decided by reading the live DOM and comparing it with the baseline captured at the
/// last load or save. That is deliberate: an edit-then-undo leaves the document byte-identical to
/// where it started, and a dialog that fires on a document you have not actually changed teaches
/// people to dismiss it without reading, which is exactly when it stops protecting anything.
///
/// The answer is asynchronous (the DOM read is), so `proceed` runs from the callback rather than
/// this function returning a verdict.
fn confirm_if_modified<F: Fn() + 'static>(
    window: &adw::ApplicationWindow,
    view: &webkit6::WebView,
    state: &Rc<RefCell<State>>,
    proceed: F,
) {
    let window = window.clone();
    let state = state.clone();
    view.evaluate_javascript(
        "document.documentElement.outerHTML",
        None,
        None,
        gtk::gio::Cancellable::NONE,
        move |res| {
            let current = res.map(|v| v.to_str().to_string()).unwrap_or_default();
            // An empty baseline means the first load has not finished. Treat that as unmodified:
            // there is nothing on screen to lose, and blocking on it would be a phantom prompt.
            let unmodified = {
                let s = state.borrow();
                s.baseline.is_empty() || current == s.baseline
            };
            if unmodified {
                proceed();
                return;
            }

            let name = doc_title(&state.borrow().path);
            let dlg = adw::MessageDialog::new(
                Some(&window),
                Some("Close without saving?"),
                Some(&format!("“{name}” has changes that have not been saved. They will be lost.")),
            );
            // Measured, not assumed: a stacked AdwMessageDialog renders responses in REVERSE order
            // of addition, so adding cancel first put the destructive answer on top. Discard goes
            // in first, which lands Cancel where the eye starts.
            dlg.add_response("discard", "Close without saving");
            dlg.add_response("cancel", "Cancel");
            dlg.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            // Enter and Escape both land on Cancel: the safe answer is the one you get by reflex.
            dlg.set_default_response(Some("cancel"));
            dlg.set_close_response("cancel");
            dlg.connect_response(None, move |dlg, resp| {
                dlg.close();
                if resp == "discard" {
                    proceed();
                }
            });
            dlg.present();
        },
    );
}

/// Paper size and the four margins — **this document's**, with the option to make them the default
/// for new ones.
fn page_setup_dialog(
    window: &adw::ApplicationWindow,
    view: &webkit6::WebView,
    state: &Rc<RefCell<State>>,
    settings: &Rc<Settings>,
) {
    let dlg = adw::Window::builder()
        .transient_for(window)
        .modal(true)
        .title("Page setup")
        .default_width(420)
        .build();

    let cur = state.borrow().setup;
    let group = adw::PreferencesGroup::new();
    group.set_description(Some("Applies to this document, which carries it in the file."));

    let paper = adw::ComboRow::new();
    paper.set_title("Paper size");
    paper.set_model(Some(&gtk::StringList::new(
        &Paper::ALL.iter().map(|p| p.label()).collect::<Vec<_>>(),
    )));
    paper.set_selected(cur.paper.index());
    group.add(&paper);

    let mk = |title: &str, v: f64| {
        let r = adw::SpinRow::with_range(0.0, 60.0, 1.0);
        r.set_title(title);
        r.set_value(v);
        r
    };
    let (mt, mr, mb, ml) = (
        mk("Top margin (mm)", cur.top),
        mk("Right margin (mm)", cur.right),
        mk("Bottom margin (mm)", cur.bottom),
        mk("Left margin (mm)", cur.left),
    );
    for r in [&mt, &mr, &mb, &ml] {
        group.add(r);
    }

    let as_default = adw::SwitchRow::new();
    as_default.set_title("Also make this the default");
    as_default.set_subtitle("New documents will start with this paper and these margins");
    group.add(&as_default);

    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label("Cancel");
    {
        let dlg = dlg.clone();
        cancel.connect_clicked(move |_| dlg.close());
    }
    {
        let dlg = dlg.clone();
        let state = state.clone();
        let view = view.clone();
        let settings = settings.clone();
        let (paper, mt, mr, mb, ml, as_default) =
            (paper.clone(), mt.clone(), mr.clone(), mb.clone(), ml.clone(), as_default.clone());
        apply.connect_clicked(move |_| {
            let setup = PageSetup {
                paper: Paper::from_index(paper.selected()),
                top: mt.value(),
                right: mr.value(),
                bottom: mb.value(),
                left: ml.value(),
            };
            state.borrow_mut().setup = setup;
            if as_default.is_active() {
                setup.save_as_default(&settings);
            }
            // Re-style the live document so the editing column matches the new page width, and
            // record the geometry in the document itself. Done by rewriting the base block rather
            // than reloading, so the text and the undo stack survive -- reloading here would
            // silently discard unsaved work.
            let base = Base::from_settings(&settings);
            docstyle::inject_base(&view, &base, setup);
            set_page_meta(&view, setup);
            dlg.close();
        });
    }

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&apply);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.append(&group);
    content.append(&buttons);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&content));
    dlg.set_content(Some(&tv));
    dlg.present();
}

/// Record the page geometry in the live document, so it is there when the DOM is serialised.
fn set_page_meta(view: &webkit6::WebView, setup: PageSetup) {
    let script = format!(
        "(function (page) {{
           let m = document.querySelector('meta[name=\"{meta}\"]');
           if (!m) {{
             m = document.createElement('meta');
             m.setAttribute('name', '{meta}');
             document.head.insertBefore(m, document.head.firstChild);
           }}
           m.setAttribute('content', page);
         }})({page})",
        meta = page::PAGE_META,
        page = js::string(&setup.to_meta()),
    );
    view.evaluate_javascript(&script, None, None, gtk::gio::Cancellable::NONE, |_| {});
}
