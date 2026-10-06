# termy_x_cli

The `x` binary. It is a thin entry point over `termy_x`.

## Owner

This crate owns the standalone `x` executable shipped beside Termy. Command behavior lives in `termy_x` so `termy x` and `x` stay the same. It must not link GPUI.

## Validation

```sh
cargo test -p termy_x
cargo run -p termy_x_cli --bin x -- doctor
cargo run -p termy_x_cli --bin x -- splash
cargo run -p termy_x_cli --bin x -- post --dry-run "hello world"
```

## Forbidden Dependencies

- `gpui`
- `gpui-kit`
- `termy` / `crates/desktop_app`
