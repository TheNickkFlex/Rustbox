// Reusable native dialog helpers (no external toolkits).
//
// The workspace-rename dialog is implemented as a stateful, managed window in
// `screen/mod.rs` (see `show_rename_dialog` / `handle_dialog_key`). It must be
// a *managed* window — termux-x11 only delivers the soft keyboard to windows
// the WM has actually framed and focused, which is why a plain override-redirect
// dialog never received key events.
