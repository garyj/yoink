# Licence and acknowledgements

yoink is copyright 2026 garyj and contributors, and is distributed under
GPL-3.0-or-later. See [LICENSE](LICENSE) for the full terms.

[CopyQ](https://github.com/hluk/CopyQ), by Lukas Holecek and contributors,
informed yoink's behavior. Existing adaptation notes identify these references:

- The X11 activation handling in `src-tauri/src/lib.rs` references
  [`X11PlatformWindow::raise()`](https://github.com/hluk/CopyQ/blob/master/src/platform/x11/x11platformwindow.cpp).
- The retention and accented-search scenarios in
  `crates/yoink-core/tests/scenarios.rs` reference
  [CopyQ's item tests](https://github.com/hluk/CopyQ/blob/master/src/tests/tests_items.cpp).

Those upstream files are GPL-3.0-or-later. yoink uses the same licence
conservatively to cover potential adaptations retained here. Referencing an
application's behavior alone is not the reason for this choice.

The Rust implementation uses Tauri, arboard, rusqlite, ksni, and other
dependencies. The frontend uses Svelte and Tauri's JavaScript API. These projects
retain their own licences and copyrights. Release archives include their
licence texts in `THIRD_PARTY_LICENSES.html` and `FRONTEND_LICENSES.txt`.

The yoink icon is an original geometric drawing in `assets/yoink.svg`, covered
by the project licence.
