# Dependency review

The release checks use the committed Cargo and Bun lockfiles. The supported
build toolchain is Rust 1.98.0 and Bun 1.4.0. These are tested versions, not
a claim that every older Rust toolchain fails.

The review on 6 September 2026 found these upstream constraints:

| Dependency | Finding | Assessment |
| --- | --- | --- |
| `cookie 0.6.0`, through SvelteKit | [GHSA-pxg6-pf52-xh8x](https://github.com/advisories/GHSA-pxg6-pf52-xh8x), low severity | yoink ships static frontend assets with no SvelteKit server or cookie-writing endpoint. The vulnerable server dependency is absent from the client bundle. Keep the advisory visible until SvelteKit removes the affected dependency. |
| `glib 0.18.5`, through Tauri's GTK3 stack | [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html), unsound string iterator | No affected iterator call was found in yoink, Tauri, Tao, or Wry. Full transitive reachability is unknown. The fix is in GLib 0.20+, which cannot be forced into the GTK3 dependency tree as a compatible patch. |
| GTK3 and `unic-*` dependencies | Upstream maintenance advisories | Retained as Tauri dependencies for this Linux desktop app. Track upstream support instead of silently suppressing audit results. |

`bun audit` therefore reports a known advisory. Compilation and tests do not
establish the absence of dependency vulnerabilities. Recheck advisories and
the affected call paths before each release.

`cargo-about` generates Rust dependency notices from the locked graph.
The Vite build collects notices for packages whose modules enter the output.
`licenses/tauri-api-MIT.txt` preserves the Tauri API's source copyright notice
and the upstream MIT terms because its npm package omits the root licence file.
The bundled tslib notice comes directly from the API package's source header.
