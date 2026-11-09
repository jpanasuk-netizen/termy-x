# termy_x_cli

The `x` binary. It is a thin entry point over `termy_x`.

## Owner

This crate owns the standalone `x` executable shipped beside Termy. Command behavior lives in `termy_x` so `termy x` and `x` stay the same. It must not link GPUI.

## Validation

```sh
cargo test -p termy_x
cargo run -p termy_x_cli --bin termy-xcli -- doctor
cargo run -p termy_x_cli --bin termy-xcli -- splash
cargo run -p termy_x_cli --bin termy-xcli -- post --dry-run "hello world"
```

## Forbidden Dependencies

- `gpui`
- `gpui-kit`
- `termy` / `crates/desktop_app`
