# Third-party patches

## cookie (0.18.1)

Vendored from crates.io `cookie` 0.18.1 with a minimal patch to
`src/expiration.rs`: replace blanket `impl<T: Into<Option<OffsetDateTime>>> From<T>`
with explicit `From<Option<OffsetDateTime>>` and `From<OffsetDateTime>` impls.

## time (0.3.41)

Vendored from crates.io `time` 0.3.41 to pin below 0.3.47, which introduces a
`From<HourBase>` impl that conflicts with blanket `From<T>` impls in `cookie` and
`tauri-utils` on Rust 1.94.

Remove these vendored crates when upstream publishes fixes or dependency trees update.

## tauri (2.11.2)

Vendored from crates.io `tauri` 2.11.2 with minimal patches to replace blanket
`From<T>` impls in `src/event/mod.rs` and `src/ipc/mod.rs` (Rust 1.94 / time 0.3.48).
