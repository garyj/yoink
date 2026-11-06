# yoink agent rules

Resident agent: **Baron von Yoinkenstein**. `CLAUDE.md` is a symlink to
`AGENTS.md`; edit this file.

- Use Bun and the committed lockfile. Build and test commands are in the README
  and `justfile`.
- Keep GUI and clipboard dependencies out of `yoink-core`. Its tests must run
  without GTK or WebKit development packages.
- Keep `crates/yoink-clipboard` as the only runtime owner of
  `arboard::Clipboard` in each front end. On X11, dropping that owner makes
  later paste requests lose the clipboard contents.
- Use a desktop keybinding that runs `yoink` to summon the existing instance.
  Do not add an in-app global shortcut. X11 grab conflicts and window-manager
  focus rules make that path unreliable.
- Do not add hide-on-blur behavior without an explicit request. Cinnamon's
  focus-follows-mouse behavior produces transient focus changes during pointer
  movement and window remapping.
- Refresh on visibility changes, not focus gains. Refreshing on focus causes
  flicker whenever the pointer enters the window.
- Build release executables with `bun run tauri build --no-bundle -- --locked`.
  Bare `cargo build --release` omits Tauri's embedded-asset feature and leaves
  the webview trying to contact the development server.
- In `yoink-gpui`, show and hide the window only through `src/x11.rs`. GPUI
  cannot unmap a window on X11, and `Window::activate_window` sends
  `CURRENT_TIME`, which Cinnamon's `focus-new-windows 'smart'` refuses. The
  window then maps behind the focused app.
- Keep the `yoink-gpui` window background transparent. gpui-component reserves
  a 20 px client-side shadow inset, and an opaque window paints it black.
- A running instance records the current clipboard when it starts. Take
  screenshots for outside review from a private Xvfb display with seeded
  history, never from the desktop session.
