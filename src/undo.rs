//! Undo that covers **the last action**, whatever kind it was.
//!
//! ## The problem
//!
//! WebKit keeps its own undo stack, and it knows about typing, pasting and the editing commands the
//! toolbar issues. It knows nothing about a style change, because a style change is not an edit to
//! the document's text — it is a rewrite of the `<style>` block's contents, done through
//! `evaluate_javascript`. So before this existed, Piers' sequence went wrong at the last step:
//!
//! > a1 type "hello world" · a2 copy and paste it 3 times · a3 style sidebar: all paragraphs bold ·
//! > a4 pick one paragraph and set it underlined · **a5 undo × 2**
//!
//! Undo × 2 would have thrown away two of the *pastes* and left both style changes applied. What it
//! must do is take back a4 and then a3, leaving the four paragraphs as they were typed.
//!
//! ## How the two stacks are kept in order
//!
//! There is no notification when WebKit records an undo step, so the two stacks cannot simply be
//! merged. Instead each style step remembers a **fingerprint of the document's content** at the
//! moment it was applied — a hash of the body with editing markers and `wg-iN` handles stripped
//! out, so minting a handle does not read as an edit and only real content changes move it.
//!
//! Undo then asks one question: *has the content changed since the newest style step?*
//!
//! - **No** — the style step is the most recent thing that happened, so take it back.
//! - **Yes** — something was typed since, so that is the more recent action: hand over to WebKit.
//!
//! For Piers' sequence the fingerprint after a2 is unchanged by a3 and a4, so both undos take the
//! style steps, newest first. Type a word between them and the first undo correctly goes to the
//! text instead.
//!
//! Redo is the mirror image, and needs no bookkeeping to invalidate itself: if you type after
//! undoing a style change, the fingerprint no longer matches and redo goes to WebKit instead, which
//! is exactly right.
//!
//! ## The one corner it does not cover
//!
//! Text, then a style change, then more text, then undo held down: the text undoes newest-first as
//! it should, but WebKit's stack runs on past the style boundary, so the older text comes back
//! before the style step does. The end state after undoing everything is the same, only the middle
//! differs. When WebKit reports it has nothing left, the style steps are taken regardless, so no
//! step is ever stranded.

use std::cell::RefCell;
use std::rc::Rc;

use webkit6::prelude::*;

use crate::docstyle::{self, CustomStyles};
use crate::State;

/// One style change, and the content fingerprint it was made against.
pub struct StyleStep {
    pub before: CustomStyles,
    pub after: CustomStyles,
    pub fingerprint: String,
}

/// The script that reads the content fingerprint. Safe before the helpers are injected.
pub const FINGERPRINT_JS: &str = "window.wgCursor ? window.wgCursor.fingerprint() : ''";

/// Record a style change so it can be taken back. Applying anything new discards the redo stack,
/// same as any editor.
pub fn record(state: &Rc<RefCell<State>>, step: StyleStep) {
    let mut s = state.borrow_mut();
    s.undo.push(step);
    s.redo.clear();
}

pub fn undo(view: &webkit6::WebView, state: &Rc<RefCell<State>>, changed: Rc<dyn Fn()>) {
    step(view, state, true, changed);
}

pub fn redo(view: &webkit6::WebView, state: &Rc<RefCell<State>>, changed: Rc<dyn Fn()>) {
    step(view, state, false, changed);
}

fn step(
    view: &webkit6::WebView,
    state: &Rc<RefCell<State>>,
    undoing: bool,
    changed: Rc<dyn Fn()>,
) {
    let view = view.clone();
    let state = state.clone();
    view.clone().evaluate_javascript(
        FINGERPRINT_JS,
        None,
        None,
        gtk::gio::Cancellable::NONE,
        move |res| {
            let fingerprint = res.map(|v| v.to_str().to_string()).unwrap_or_default();

            // Does the stack we are about to pop from hold the most recent action?
            let take_style = {
                let s = state.borrow();
                let stack = if undoing { &s.undo } else { &s.redo };
                match stack.last() {
                    // Either nothing has been typed since, or WebKit has nothing left to give —
                    // the second case is what stops a style step ever being stranded.
                    Some(top) => top.fingerprint == fingerprint || !webkit_has(&view, undoing),
                    None => false,
                }
            };

            if !take_style {
                view.execute_editing_command(if undoing { "Undo" } else { "Redo" });
                return;
            }

            let Some(step) = ({
                let mut s = state.borrow_mut();
                if undoing { s.undo.pop() } else { s.redo.pop() }
            }) else {
                return;
            };

            let target = if undoing { step.before.clone() } else { step.after.clone() };
            docstyle::inject_custom(&view, &docstyle::custom_css(&target));
            {
                let mut s = state.borrow_mut();
                s.custom = target;
                if undoing {
                    s.redo.push(step);
                } else {
                    s.undo.push(step);
                }
            }
            // The overrides just changed underneath the style sidebar. Leaving its rows showing the
            // old values would let the next Apply put back exactly what was undone.
            changed();
        },
    );
}

/// Whether WebKit still has anything on its own stack.
fn webkit_has(view: &webkit6::WebView, undoing: bool) -> bool {
    match view.editor_state() {
        Some(editor) => {
            if undoing {
                editor.is_undo_available()
            } else {
                editor.is_redo_available()
            }
        }
        // No editor state means nothing is being edited; do not claim there is history.
        None => false,
    }
}
