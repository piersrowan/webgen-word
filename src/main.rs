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

mod doc;
mod docstyle;
mod js;
mod page;
mod sanitise;
mod settings;
mod sidebar;

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

    for (icon, tip, cmd) in [
        ("edit-undo-symbolic", "Undo", "Undo"),
        ("edit-redo-symbolic", "Redo", "Redo"),
    ] {
        let b = tool(icon, tip);
        let view = view.clone();
        b.connect_clicked(move |_| view.execute_editing_command(cmd));
        fmt.append(&b);
    }

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
    let load_into = {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        let settings = settings.clone();
        let say = say.clone();
        Rc::new(move |path: Option<PathBuf>| {
            let base = Base::from_settings(&settings);
            let bytes = path.as_ref().map(|p| (p.clone(), std::fs::read(p)));

            let (html, setup, path, report) = match bytes {
                // Opened a file that reads and looks like a document.
                Some((p, Ok(raw))) if doc::looks_like_html(&raw) => {
                    let source = String::from_utf8_lossy(&raw).to_string();
                    // Cut script and imported assets out of the SOURCE. Doing this after loading
                    // would be doing it after the script had already run.
                    let (clean, report) = sanitise::clean(&source, p.parent());
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
            }
            if let Some(summary) = report.summary() {
                say(&summary);
            }
        })
    };

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
    let save_to = {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        let say = say.clone();
        Rc::new(move |path: PathBuf| {
            let window = window.clone();
            let state = state.clone();
            let say = say.clone();
            let title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let page_meta = state.borrow().setup.to_meta();
            // Three fixups in the DOM before serialising: drop the editing-only outline class, give
            // the document a `<title>` matching its file name if it is still the placeholder (0.2.0
            // fixed the *window* title and left the document's own saying "Untitled" forever), and
            // record the page geometry so the file describes the shape it was written for.
            let script = format!(
                "(function (title, page) {{
                   /* The editing-only markers must not reach the file -- but they must survive the
                      save. Removing them outright was a real defect: after one Ctrl+S the style
                      sidebar had no selected element any more and silently stopped responding. So
                      they come off, the document is serialised, and they go straight back on. */
                   const marked = Array.from(document.querySelectorAll('.wg-selected, .{cursor}'));
                   const before = marked.map(function (el) {{ return el.getAttribute('class'); }});
                   marked.forEach(function (el) {{
                     el.classList.remove('wg-selected');
                     el.classList.remove('{cursor}');
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
                   const html = document.documentElement.outerHTML;
                   marked.forEach(function (el, i) {{ el.setAttribute('class', before[i]); }});
                   return html;
                 }})({title}, {page})",
                meta = page::PAGE_META,
                cursor = docstyle::CURSOR_CLASS,
                title = js::string(&title),
                page = js::string(&page_meta),
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
                    let (clean, report) = sanitise::clean(&html, path.parent());
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

                    if let Some(summary) = report.summary() {
                        say(&summary);
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
        Rc::new(move || {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("HTML document"));
            filter.add_pattern("*.html");
            filter.add_pattern("*.htm");
            let list = gtk::gio::ListStore::new::<gtk::FileFilter>();
            list.append(&filter);
            let current = state.borrow().path.clone();
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
                        save_to(with_html_extension(p));
                    }
                }
            });
        })
    };

    let save = {
        let save_to = save_to.clone();
        let save_as = save_as.clone();
        let state = state.clone();
        Rc::new(move || {
            // Read the path out and let the borrow go before saving: holding a RefCell borrow across
            // a call that may re-enter it is a panic waiting for someone to make the callback
            // synchronous.
            let path = state.borrow().path.clone();
            match path {
                Some(p) => save_to(p),
                None => save_as(),
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
        open_b.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("HTML document"));
            filter.add_pattern("*.html");
            filter.add_pattern("*.htm");
            let list = gtk::gio::ListStore::new::<gtk::FileFilter>();
            list.append(&filter);
            let d = gtk::FileDialog::builder().title("Open document").filters(&list).build();
            let load_into = load_into.clone();
            let window2 = window.clone();
            // Opening replaces what is on screen, so it asks the same question Close does.
            confirm_if_modified(&window, &view, &state, move || {
                let load_into = load_into.clone();
                d.open(Some(&window2), gtk::gio::Cancellable::NONE, move |res| {
                    if let Ok(f) = res {
                        load_into(f.path());
                    }
                });
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
        let say = say.clone();
        image_b.connect_clicked(move |_| insert_picture(&window, &view, &say));
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
            sidebar_select(vx, vy);
        });
        window.add_controller(clicks);
    }
    {
        let set_open = sidebar.set_open.clone();
        style_b.connect_toggled(move |b| set_open(b.is_active()));
    }
    {
        // Closing from inside the sidebar has to pop the header toggle back out.
        let style_b = style_b.clone();
        let is_open = sidebar.is_open.clone();
        sidebar.root.connect_child_revealed_notify(move |_| {
            style_b.set_active(is_open());
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
                return glib::Propagation::Proceed;
            }
            let w2 = w.clone();
            confirm_if_modified(w, &view, &state, move || w2.destroy());
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
        Rc::new(move || {
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
    menu_b.set_menu_model(Some(&menu));

    // --- keyboard ---------------------------------------------------------------------------------
    // CAPTURE phase, so these beat the WebView. Only bindings WebKit does NOT already provide are
    // added: it handles Ctrl+B/I/U, Ctrl+Z/Y and the clipboard itself, and binding those again here
    // would fire the command twice and un-toggle it.
    {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let view_k = view.clone();
        let new_click = new_b.clone();
        let open_click = open_b.clone();
        let print = print.clone();
        let save = save.clone();
        let save_as = save_as.clone();
        let close_document = close_document.clone();
        let brk_click = brk.clone();
        keys.connect_key_pressed(move |_, key, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            use gtk::gdk::Key;
            match (ctrl, shift, key) {
                (true, true, Key::S | Key::s) => save_as(),
                (true, false, Key::n) => new_click.emit_clicked(),
                (true, false, Key::o) => open_click.emit_clicked(),
                (true, false, Key::s) => save(),
                (true, false, Key::p) => print(),
                (true, false, Key::w) => close_document(),
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
    window.present();
}

/// Translate a click on the window into the WebView's own coordinates, or `None` when it landed
/// somewhere else — the toolbar, the sidebar, the header.
fn window_to_view(view: &webkit6::WebView, x: f64, y: f64) -> Option<(f64, f64)> {
    let root = view.root()?;
    let (vx, vy) = root.translate_coordinates(view, x, y)?;
    let inside = vx >= 0.0 && vy >= 0.0 && vx < view.width() as f64 && vy < view.height() as f64;
    inside.then_some((vx, vy))
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

/// Pick a picture and put it in the document — **embedded**, not referenced.
///
/// A document that references `/home/you/photo.jpg` breaks the moment it is emailed, and so does its
/// PDF. Embedding is what makes "the file on disk is a standalone .html" true rather than aspirational.
fn insert_picture(
    window: &adw::ApplicationWindow,
    view: &webkit6::WebView,
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
    d.open(Some(window), gtk::gio::Cancellable::NONE, move |res| {
        let Ok(file) = res else { return };
        let Some(path) = file.path() else { return };
        // Reuse the sanitiser's own embedding, so a picture inserted here and a picture found in an
        // opened document go through exactly the same code.
        let markup = format!("<img src=\"{}\" class=\"wg-center\">", path.display());
        let (clean, report) = sanitise::clean(&markup, path.parent());
        if report.embedded == 0 {
            say(&format!("{} could not be read, or is too large to embed.", path.display()));
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
