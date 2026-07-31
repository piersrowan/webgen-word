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
//! ## The one thing that is not obvious
//!
//! **CSS `@page` does nothing on our print path.** Measured before this app was written: a document
//! asking for A4 with 45mm/40mm margins printed as US Letter with 10mm/7mm — GTK's defaults. WebKit's
//! GTK print backend takes page geometry from `gtk::PageSetup` and ignores the stylesheet entirely.
//! That is why `page.rs` exists and why Page setup is a toolbar button rather than a CSS comment.

mod doc;
mod page;

use adw::prelude::*;
use gtk::glib;
use page::{PageSetup, Paper};
use std::cell::RefCell;
use std::rc::Rc;
use webkit6::prelude::*;

const APP_ID: &str = "com.webgen.Word";

/// Everything the window needs to know about the open document.
struct State {
    /// Where it came from / goes back to. `None` until first save.
    path: Option<std::path::PathBuf>,
    setup: PageSetup,
    /// The document's HTML as it stood at the last load or save, read back OUT OF THE DOM rather
    /// than off disk. WebKit normalises markup on parse, so the file's own bytes are not a usable
    /// baseline -- comparing against them would report "modified" on a document nobody touched.
    /// This is what makes "close without saving?" ask only when there is something to lose.
    baseline: String,
}

/// What goes in the title bar: the FILE NAME, not the document's `<title>`.
///
/// It used to be `<title>`, which meant a new document said "Untitled" forever -- saving it as
/// `cv.html` changed nothing on screen, because the heading inside the file still said Untitled.
/// Every word processor titles the window after the file; that is the thing you are looking for
/// when you scan a taskbar.
fn doc_title(path: &Option<std::path::PathBuf>) -> String {
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
    app.connect_activate(|a| {
        build(a, None);
    });
    app.connect_open(|a, files, _| {
        for f in files {
            build(a, f.path());
        }
    });
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

fn build(app: &adw::Application, open: Option<std::path::PathBuf>) {
    let state = Rc::new(RefCell::new(State {
        path: open.clone(),
        setup: PageSetup::default(),
        baseline: String::new(),
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

    // --- toolbar ---------------------------------------------------------------------------------
    let header = adw::HeaderBar::new();

    let new_b = tool("document-new-symbolic", "New document");
    let open_b = tool("document-open-symbolic", "Open…");
    let save_b = tool("document-save-symbolic", "Save");
    // Close the DOCUMENT, not the window: the window stays and gets a fresh blank one, so there is
    // a way to put a file down without either quitting the app or leaving it open by accident.
    let close_b = tool("window-close-symbolic", "Close document  (Ctrl+W)");
    header.pack_start(&new_b);
    header.pack_start(&open_b);
    header.pack_start(&save_b);
    header.pack_start(&close_b);

    let setup_b = tool("document-properties-symbolic", "Page setup — paper size and margins");
    let print_b = tool("document-print-symbolic", "Print / export PDF");
    header.pack_end(&print_b);
    header.pack_end(&setup_b);

    // Formatting row. `execute_editing_command` names are WebKit's own.
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

    // A page break is the one thing a CV genuinely needs and HTML has no key for. It goes in as a
    // styled empty div; the stylesheet turns it into `break-before: page`.
    // Icon: `go-bottom-symbolic` (push down to a bar). It was `format-justify-left-symbolic`, which
    // was simply wrong -- that is the align-left glyph, and it now belongs to the button above.
    let brk = tool("go-bottom-symbolic", "Insert page break  (Ctrl+Return)");
    {
        let view = view.clone();
        brk.connect_clicked(move |_| {
            view.evaluate_javascript(
                "document.execCommand('insertHTML',false,'<div class=\"pagebreak\"></div><p><br></p>')",
                None,
                None,
                gtk::gio::Cancellable::NONE,
                |_| {},
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
                &format!("document.execCommand('formatBlock',false,'{tag}')"),
                None,
                None,
                gtk::gio::Cancellable::NONE,
                |_| {},
            );
        });
    }

    // --- document load ---------------------------------------------------------------------------
    let load_into = {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        Rc::new(move |path: Option<std::path::PathBuf>| {
            let setup = state.borrow().setup;
            match path.as_ref().and_then(|p| std::fs::read(p).ok()) {
                Some(bytes) if doc::looks_like_html(&bytes) => {
                    let html = String::from_utf8_lossy(&bytes).to_string();
                    // base URI = the file's own directory, so relative <img src> resolves.
                    let base = path
                        .as_ref()
                        .and_then(|p| p.parent())
                        .map(|d| format!("file://{}/", d.display()));
                    view.load_html(&html, base.as_deref());
                    window.set_title(Some(&format!("{} — Word", doc_title(&path))));
                    state.borrow_mut().path = path;
                }
                // Either the file would not read, or it read but is not HTML. Both land on a blank
                // document with no path, so a later Save cannot silently overwrite the file we
                // declined to open.
                _ => {
                    view.load_html(&doc::blank(setup), None);
                    window.set_title(Some("Untitled — Word"));
                    state.borrow_mut().path = None;
                }
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

    // --- save -------------------------------------------------------------------------------------
    // The document is read back out of the live DOM, so what saves is exactly what is on screen.
    let save_to = {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        Rc::new(move |path: std::path::PathBuf| {
            let window = window.clone();
            let state = state.clone();
            view.evaluate_javascript(
                "document.documentElement.outerHTML",
                None,
                None,
                gtk::gio::Cancellable::NONE,
                move |res| {
                    let Ok(v) = res else { return };
                    let html = v.to_str();
                    if let Err(e) = std::fs::write(&path, html.as_bytes()) {
                        eprintln!("webgen-word: could not save {}: {e}", path.display());
                        return;
                    }
                    let path = Some(path);
                    window.set_title(Some(&format!("{} — Word", doc_title(&path))));
                    // Saved == not modified. Without this the very next Close would still claim
                    // unsaved changes on a document that had just been written to disk.
                    let mut s = state.borrow_mut();
                    s.baseline = html.to_string();
                    s.path = path;
                },
            );
        })
    };

    {
        let save_to = save_to.clone();
        let state = state.clone();
        let window = window.clone();
        save_b.connect_clicked(move |_| {
            if let Some(p) = state.borrow().path.clone() {
                save_to(p);
                return;
            }
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("HTML document"));
            filter.add_pattern("*.html");
            filter.add_pattern("*.htm");
            let list = gtk::gio::ListStore::new::<gtk::FileFilter>();
            list.append(&filter);
            let d = gtk::FileDialog::builder()
                .title("Save document")
                .initial_name("document.html")
                .filters(&list)
                .build();
            let save_to = save_to.clone();
            d.save(Some(&window), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res {
                    if let Some(p) = f.path() {
                        save_to(p);
                    }
                }
            });
        });
    }

    {
        let load_into = load_into.clone();
        let window = window.clone();
        open_b.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("HTML document"));
            filter.add_pattern("*.html");
            filter.add_pattern("*.htm");
            let list = gtk::gio::ListStore::new::<gtk::FileFilter>();
            list.append(&filter);
            let d = gtk::FileDialog::builder().title("Open document").filters(&list).build();
            let load_into = load_into.clone();
            d.open(Some(&window), gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(f) = res {
                    load_into(f.path());
                }
            });
        });
    }

    // New opens a NEW WINDOW. It used to reload the current view, which silently discarded whatever
    // was on screen -- there is no autosave, so that was a data-loss button wearing a "new" label.
    {
        let app = app.clone();
        new_b.connect_clicked(move |_| build(&app, None));
    }

    // --- close ------------------------------------------------------------------------------------
    // Close puts the FILE down and leaves the window open on a fresh blank document. Quitting to get
    // rid of a file, or opening another one just to displace it, were the only ways to do this.
    {
        let load_into = load_into.clone();
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        close_b.connect_clicked(move |_| {
            let load_into = load_into.clone();
            confirm_if_modified(&window, &view, &state, move || load_into(None));
        });
    }

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

    // --- page setup --------------------------------------------------------------------------------
    {
        let state = state.clone();
        let window = window.clone();
        let view = view.clone();
        setup_b.connect_clicked(move |_| page_setup_dialog(&window, &view, &state));
    }

    // --- print ---------------------------------------------------------------------------------------
    {
        let view = view.clone();
        let window = window.clone();
        let state = state.clone();
        print_b.connect_clicked(move |_| {
            let op = webkit6::PrintOperation::new(&view);
            // THE important line. Without it the print uses GTK's defaults (US Letter, ~10mm) and
            // the document's own @page is ignored -- measured, see the module docs.
            op.set_page_setup(&state.borrow().setup.to_gtk());
            op.run_dialog(Some(&window));
        });
    }

    // --- keyboard ----------------------------------------------------------------------------------
    // CAPTURE phase, so these beat the WebView. Only bindings WebKit does NOT already provide are
    // added: it handles Ctrl+B/I/U, Ctrl+Z/Y and the clipboard itself, and binding those again here
    // would fire the command twice and un-toggle it.
    {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let view_k = view.clone();
        let new_click = new_b.clone();
        let open_click = open_b.clone();
        let save_click = save_b.clone();
        let print_click = print_b.clone();
        let close_click = close_b.clone();
        let brk_click = brk.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            use gtk::gdk::Key;
            match (ctrl, key) {
                (true, Key::n) => new_click.emit_clicked(),
                (true, Key::o) => open_click.emit_clicked(),
                (true, Key::s) => save_click.emit_clicked(),
                (true, Key::p) => print_click.emit_clicked(),
                (true, Key::w) => close_click.emit_clicked(),
                (true, Key::Return | Key::KP_Enter) => brk_click.emit_clicked(),
                // Tab indents, Shift+Tab outdents. In a list this is how a sub-list is made, which
                // is the behaviour every word processor has and the reason Tab does not insert a
                // tab character here.
                (false, Key::Tab) if !shift => view_k.execute_editing_command("Indent"),
                (false, Key::ISO_Left_Tab) => view_k.execute_editing_command("Outdent"),
                (false, Key::Tab) if shift => view_k.execute_editing_command("Outdent"),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        window.add_controller(keys);
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&fmt);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let scroller = gtk::ScrolledWindow::builder().child(&view).vexpand(true).build();
    content.append(&scroller);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&content));
    window.set_content(Some(&tv));
    window.present();
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
            dlg.add_response("cancel", "Cancel");
            dlg.add_response("discard", "Close without saving");
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

/// Paper size and the four margins. Applying rewrites the document's stylesheet so the on-screen
/// column matches the printed page — otherwise you would be typing to one width and printing another.
fn page_setup_dialog(
    window: &adw::ApplicationWindow,
    view: &webkit6::WebView,
    state: &Rc<RefCell<State>>,
) {
    let dlg = adw::Window::builder()
        .transient_for(window)
        .modal(true)
        .title("Page setup")
        .default_width(400)
        .build();

    let cur = state.borrow().setup;
    let group = adw::PreferencesGroup::new();

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
        let (paper, mt, mr, mb, ml) = (paper.clone(), mt.clone(), mr.clone(), mb.clone(), ml.clone());
        apply.connect_clicked(move |_| {
            {
                let mut s = state.borrow_mut();
                s.setup = PageSetup {
                    paper: Paper::from_index(paper.selected()),
                    top: mt.value(),
                    right: mr.value(),
                    bottom: mb.value(),
                    left: ml.value(),
                };
            }
            // Re-style the live document so the editing column matches the new page width. Done by
            // replacing the <style> contents rather than reloading, so the text and the undo stack
            // survive -- reloading here would silently discard unsaved work.
            let css = doc::stylesheet(state.borrow().setup);
            let js = format!(
                "(function(){{let s=document.querySelector('style');\
                  if(!s){{s=document.createElement('style');document.head.appendChild(s);}}\
                  s.textContent={};}})()",
                js_string(&css)
            );
            view.evaluate_javascript(&js, None, None, gtk::gio::Cancellable::NONE, |_| {});
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

/// A JS string literal. The stylesheet is ours, but it still goes through a quoting function rather
/// than being pasted into a script -- a stylesheet that grew a backtick or a newline would otherwise
/// break the statement, and the habit of interpolating raw text into JS is one worth not having.
fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
